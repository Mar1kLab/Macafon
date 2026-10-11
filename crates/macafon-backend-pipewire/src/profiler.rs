//! PipeWire cycle measurements arrive on the control loop, never the audio thread.
use crate::Error;
use pipewire::{self as pw, spa::pod::Pod};
use std::{ffi::c_void, ptr::NonNull};

pub(crate) struct Profiler {
    proxy: NonNull<pw::sys::pw_proxy>,
    hook: Box<pw::spa::sys::spa_hook>,
    _events: Box<pw::sys::pw_profiler_events>,
    _data: Box<Data>,
    _registry: pw::registry::RegistryRc,
}

struct Data {
    node_id: u32,
    last_finish_ns: u64,
    next_position: Option<(u32, u64)>,
    callback: Box<dyn FnMut(u64, u64)>,
    /// Set for a load sampler: the driver info of cycles that ran `node_id`.
    load: Option<Box<dyn FnMut(DriverLoad)>>,
}

/// `SPA_PROFILER_info` of the driver that runs a node (Doctor, §9.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriverLoad {
    /// DSP load of the driver's cycles, fast/medium/slow averages, 0…1.
    pub cpu_load: [f32; 3],
    /// Xruns of the driver since it started.
    pub xrun_count: u32,
}

#[derive(Clone, Copy)]
struct Cycle {
    elapsed_ns: u64,
    budget_ns: u64,
    finish_ns: u64,
    driver_id: u32,
    position: u64,
    frames: u64,
}

impl Data {
    fn observe(&mut self, cycle: Cycle) {
        if cycle.finish_ns > self.last_finish_ns {
            if self.next_position != Some((cycle.driver_id, cycle.position)) {
                // Clock positions identify actual cycles, including across quantum changes.
                // Sampling gaps and driver changes cannot prove consecutive violations.
                (self.callback)(0, cycle.budget_ns);
            }
            self.next_position = cycle
                .position
                .checked_add(cycle.frames)
                .map(|p| (cycle.driver_id, p));
            self.last_finish_ns = cycle.finish_ns;
            (self.callback)(cycle.elapsed_ns, cycle.budget_ns);
        }
    }
}

impl Profiler {
    pub(crate) fn new(
        registry: pw::registry::RegistryRc,
        id: u32,
        version: u32,
        node_id: u32,
        callback: impl FnMut(u64, u64) + 'static,
    ) -> Result<Self, Error> {
        Self::bind(
            registry,
            id,
            version,
            Box::new(Data {
                node_id,
                last_finish_ns: 0,
                next_position: None,
                callback: Box::new(callback),
                load: None,
            }),
        )
    }

    /// Report the driver load and xruns of every profiled cycle that ran `node_id`.
    pub(crate) fn load(
        registry: pw::registry::RegistryRc,
        id: u32,
        version: u32,
        node_id: u32,
        callback: impl FnMut(DriverLoad) + 'static,
    ) -> Result<Self, Error> {
        Self::bind(
            registry,
            id,
            version,
            Box::new(Data {
                node_id,
                last_finish_ns: 0,
                next_position: None,
                callback: Box::new(|_, _| {}),
                load: Some(Box::new(callback)),
            }),
        )
    }

    fn bind(
        registry: pw::registry::RegistryRc,
        id: u32,
        version: u32,
        mut data: Box<Data>,
    ) -> Result<Self, Error> {
        // SAFETY: the live registry exposes this interface; the proxy is owned below.
        let proxy = unsafe {
            pw::spa::spa_interface_call_method!(
                registry.as_raw_ptr(),
                pw::sys::pw_registry_methods,
                bind,
                id,
                pw::sys::PW_TYPE_INTERFACE_Profiler.as_ptr().cast(),
                version.min(pw::sys::PW_VERSION_PROFILER),
                0
            )
        };
        let proxy = NonNull::new(proxy.cast())
            .ok_or_else(|| Error::Message("Cannot bind PipeWire profiler".into()))?;
        // SAFETY: an unregistered spa_hook must start zero-initialized.
        let mut hook = Box::new(unsafe { std::mem::zeroed() });
        let events = Box::new(pw::sys::pw_profiler_events {
            version: pw::sys::PW_VERSION_PROFILER_EVENTS,
            profile: Some(profile),
        });
        // SAFETY: all callback storage remains at stable addresses until listener removal.
        unsafe {
            pw::sys::pw_proxy_add_object_listener(
                proxy.as_ptr(),
                &mut *hook,
                (&*events as *const pw::sys::pw_profiler_events).cast(),
                (&mut *data as *mut Data).cast(),
            );
        }
        Ok(Self {
            proxy,
            hook,
            _events: events,
            _data: data,
            _registry: registry,
        })
    }
}

impl Drop for Profiler {
    fn drop(&mut self) {
        pw::spa::utils::hook::remove(*self.hook);
        // SAFETY: this listener exclusively owns the proxy and has been removed.
        unsafe { pw::sys::pw_proxy_destroy(self.proxy.as_ptr()) };
    }
}

unsafe extern "C" fn profile(data: *mut c_void, pod: *const pw::spa::sys::spa_pod) {
    if data.is_null() || pod.is_null() {
        return;
    }
    // SAFETY: PipeWire serializes control-loop events and lends this valid pod for the callback.
    let (data, pod) = unsafe { (&mut *data.cast::<Data>(), Pod::from_raw(pod)) };
    if let Some(load) = &mut data.load {
        visit_load(pod.as_bytes(), data.node_id, load);
        return;
    }
    visit_cycles(pod.as_bytes(), data.node_id, &mut |cycle| {
        data.observe(cycle)
    });
}

// SPA pods use native-endian headers and eight-byte padding. Borrow only the fields
// needed for the target follower; decoding the entire graph into owned Values is costly.
#[derive(Clone, Copy)]
struct BorrowedPod<'a> {
    type_: u32,
    body: &'a [u8],
}

fn word(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn next_pod<'a>(bytes: &mut &'a [u8]) -> Option<BorrowedPod<'a>> {
    let size = usize::try_from(word(bytes)?).ok()?;
    let type_ = word(bytes.get(4..)?)?;
    let end = 8usize.checked_add(size)?;
    let body = bytes.get(8..end)?;
    let padded = end.checked_add(7)? & !7;
    *bytes = bytes.get(padded..)?;
    Some(BorrowedPod { type_, body })
}

impl<'a> BorrowedPod<'a> {
    fn fields(self) -> impl Iterator<Item = BorrowedPod<'a>> {
        let mut remaining = if self.type_ == pw::spa::sys::SPA_TYPE_Struct {
            self.body
        } else {
            &[]
        };
        std::iter::from_fn(move || next_pod(&mut remaining))
    }

    fn properties(self) -> impl Iterator<Item = (u32, BorrowedPod<'a>)> {
        let mut remaining = if self.type_ == pw::spa::sys::SPA_TYPE_Object {
            self.body.get(8..).unwrap_or_default()
        } else {
            &[]
        };
        std::iter::from_fn(move || {
            let key = word(remaining)?;
            remaining = remaining.get(8..)?; // key and property flags
            Some((key, next_pod(&mut remaining)?))
        })
    }

    fn int(self) -> Option<i32> {
        (self.type_ == pw::spa::sys::SPA_TYPE_Int).then_some(())?;
        Some(i32::from_ne_bytes(self.body.get(..4)?.try_into().ok()?))
    }

    fn float(self) -> Option<f32> {
        (self.type_ == pw::spa::sys::SPA_TYPE_Float).then_some(())?;
        Some(f32::from_ne_bytes(self.body.get(..4)?.try_into().ok()?))
    }

    fn long(self) -> Option<i64> {
        (self.type_ == pw::spa::sys::SPA_TYPE_Long).then_some(())?;
        Some(i64::from_ne_bytes(self.body.get(..8)?.try_into().ok()?))
    }
}

fn visit_load(bytes: &[u8], node_id: u32, callback: &mut impl FnMut(DriverLoad)) {
    let Some(root) = next_pod(&mut &*bytes) else {
        return;
    };
    for cycle in root.fields() {
        if cycle.type_ != pw::spa::sys::SPA_TYPE_Object
            || word(cycle.body) != Some(pw::spa::sys::SPA_TYPE_OBJECT_Profiler)
        {
            continue;
        }
        let runs_node = cycle.properties().any(|(key, follower)| {
            key == pw::spa::sys::SPA_PROFILER_followerBlock
                && follower.fields().next().and_then(BorrowedPod::int) == Some(node_id as i32)
        });
        if !runs_node {
            continue;
        }
        let Some((_, info)) = cycle
            .properties()
            .find(|(key, _)| *key == pw::spa::sys::SPA_PROFILER_info)
        else {
            continue;
        };
        let load = |n| info.fields().nth(n).and_then(BorrowedPod::float);
        let (Some(fast), Some(medium), Some(slow)) = (load(1), load(2), load(3)) else {
            continue;
        };
        let Some(xruns) = info.fields().nth(4).and_then(BorrowedPod::int) else {
            continue;
        };
        callback(DriverLoad {
            cpu_load: [fast, medium, slow],
            xrun_count: xruns.max(0) as u32,
        });
    }
}

fn visit_cycles(bytes: &[u8], node_id: u32, callback: &mut impl FnMut(Cycle)) {
    let Some(root) = next_pod(&mut &*bytes) else {
        return;
    };
    for cycle in root.fields() {
        if cycle.type_ != pw::spa::sys::SPA_TYPE_Object
            || word(cycle.body) != Some(pw::spa::sys::SPA_TYPE_OBJECT_Profiler)
        {
            continue;
        }
        let Some((_, clock)) = cycle
            .properties()
            .find(|(key, _)| *key == pw::spa::sys::SPA_PROFILER_clock)
        else {
            continue;
        };
        let Some(driver_id) = clock
            .fields()
            .nth(1)
            .and_then(BorrowedPod::int)
            .filter(|id| *id >= 0)
        else {
            continue;
        };
        let Some(position) = clock
            .fields()
            .nth(5)
            .and_then(BorrowedPod::long)
            .filter(|n| *n >= 0)
        else {
            continue;
        };
        let Some(rate) = clock.fields().nth(4) else {
            continue;
        };
        let Some(frames) = clock.fields().nth(6).and_then(BorrowedPod::long) else {
            continue;
        };
        if rate.type_ != pw::spa::sys::SPA_TYPE_Fraction || frames <= 0 {
            continue;
        }
        let (Some(num), Some(denom)) = (word(rate.body), rate.body.get(4..).and_then(word)) else {
            continue;
        };
        if num == 0 || denom == 0 {
            continue;
        }
        let budget_ns =
            u128::from(frames as u64) * u128::from(num) * 1_000_000_000 / u128::from(denom);
        let Ok(budget_ns) = u64::try_from(budget_ns) else {
            continue;
        };
        for (_, follower) in cycle
            .properties()
            .filter(|(key, _)| *key == pw::spa::sys::SPA_PROFILER_followerBlock)
        {
            let Some(id) = follower.fields().next().and_then(BorrowedPod::int) else {
                continue;
            };
            if id < 0 || id as u32 != node_id {
                continue;
            }
            let Some(awake_ns) = follower.fields().nth(4).and_then(BorrowedPod::long) else {
                continue;
            };
            let Some(finish_ns) = follower.fields().nth(5).and_then(BorrowedPod::long) else {
                continue;
            };
            // Node activation status 3 means finished; incomplete samples can contain stale times.
            if follower.fields().nth(6).and_then(BorrowedPod::int) != Some(3)
                || awake_ns <= 0
                || finish_ns < awake_ns
            {
                continue;
            }
            callback(Cycle {
                elapsed_ns: (finish_ns - awake_ns) as u64,
                budget_ns,
                finish_ns: finish_ns as u64,
                driver_id: driver_id as u32,
                position: position as u64,
                frames: frames as u64,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pw::spa::{
        pod::{Object, Property, PropertyFlags, Value, serialize::PodSerializer},
        utils::Fraction,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::Duration,
    };

    #[test]
    fn extracts_own_completed_cycles_with_their_actual_quantum() {
        let property = |key, value| Property {
            key,
            flags: PropertyFlags::empty(),
            value,
        };
        let cycle = |frames, id, awake, finish, position| {
            Value::Object(Object {
                type_: pw::spa::sys::SPA_TYPE_OBJECT_Profiler,
                id: 0,
                properties: vec![
                    property(
                        pw::spa::sys::SPA_PROFILER_info,
                        Value::Struct(vec![Value::Long(0)]),
                    ),
                    property(
                        pw::spa::sys::SPA_PROFILER_clock,
                        Value::Struct(vec![
                            Value::Int(0),
                            Value::Int(0),
                            Value::String(String::new()),
                            Value::Long(0),
                            Value::Fraction(Fraction {
                                num: 1,
                                denom: 48000,
                            }),
                            Value::Long(position),
                            Value::Long(frames),
                        ]),
                    ),
                    property(
                        pw::spa::sys::SPA_PROFILER_followerBlock,
                        Value::Struct(vec![
                            Value::Int(id),
                            Value::String(String::new()),
                            Value::Long(0),
                            Value::Long(0),
                            Value::Long(awake),
                            Value::Long(finish),
                            Value::Int(3),
                        ]),
                    ),
                ],
            })
        };
        let value = Value::Struct(vec![
            cycle(128, 42, 100, 900, 0),
            cycle(256, 42, 100, 1000, 128),
            cycle(128, 7, 100, 200, 2),
            cycle(128, 42, 100, 0, 2),
            cycle(0, 42, 100, 900, 2),
            cycle(128, 42, 2000, 2100, 512),
        ]);
        let bytes = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .expect("fixture pod")
            .0
            .into_inner();
        let mut measurements = [(0, 0, 0); 3];
        let mut count = 0;
        assert_no_alloc::assert_no_alloc(|| {
            visit_cycles(&bytes, 42, &mut |cycle| {
                measurements[count] = (cycle.elapsed_ns, cycle.budget_ns, cycle.position);
                count += 1;
            })
        });
        assert_eq!(count, 3);
        assert_eq!(
            measurements,
            [
                (800, 2_666_666, 0),
                (900, 5_333_333, 128),
                (100, 2_666_666, 512)
            ]
        );
        // Truncated headers and sizes are ignored rather than read beyond the buffer.
        let distinct = Rc::new(Cell::new(0));
        let observed = distinct.clone();
        let resets = Rc::new(Cell::new(0));
        let observed_resets = resets.clone();
        let mut data = Data {
            node_id: 42,
            last_finish_ns: 0,
            next_position: None,
            load: None,
            callback: Box::new(move |elapsed, _| {
                if elapsed == 0 {
                    observed_resets.set(observed_resets.get() + 1);
                } else {
                    observed.set(observed.get() + 1);
                }
            }),
        };
        for _ in 0..2 {
            visit_cycles(&bytes, 42, &mut |cycle| data.observe(cycle));
        }
        assert_eq!(distinct.get(), 3);
        assert_eq!(resets.get(), 2);
        for length in 0..bytes.len() {
            visit_cycles(&bytes[..length], 42, &mut |_| {});
        }
    }

    #[test]
    fn reads_load_and_xruns_of_the_driver_running_the_node() {
        let property = |key, value| Property {
            key,
            flags: PropertyFlags::empty(),
            value,
        };
        let cycle = |follower, xruns| {
            Value::Object(Object {
                type_: pw::spa::sys::SPA_TYPE_OBJECT_Profiler,
                id: 0,
                properties: vec![
                    property(
                        pw::spa::sys::SPA_PROFILER_info,
                        Value::Struct(vec![
                            Value::Long(7),
                            Value::Float(0.1),
                            Value::Float(0.25),
                            Value::Float(0.2),
                            Value::Int(xruns),
                        ]),
                    ),
                    property(
                        pw::spa::sys::SPA_PROFILER_followerBlock,
                        Value::Struct(vec![Value::Int(follower)]),
                    ),
                ],
            })
        };
        let value = Value::Struct(vec![cycle(7, 9), cycle(42, 3)]);
        let bytes = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .expect("fixture pod")
            .0
            .into_inner();
        let mut loads = Vec::new();
        visit_load(&bytes, 42, &mut |load| loads.push(load));
        assert_eq!(
            loads,
            [DriverLoad {
                cpu_load: [0.1, 0.25, 0.2],
                xrun_count: 3
            }]
        );
        for length in 0..bytes.len() {
            visit_load(&bytes[..length], 42, &mut |_| {});
        }
    }

    #[test]
    #[ignore = "requires tools/tests/profiler-smoke.py private audio graph"]
    fn receives_native_cycle_measurements_on_control_loop() {
        let target =
            std::env::var("MACAFON_TEST_PROFILER_NODE").expect("private graph target provided");
        let mainloop = pw::main_loop::MainLoopRc::new(None).expect("main loop");
        let connection =
            crate::Connection::with_mainloop(mainloop.clone(), true).expect("private connection");
        assert!(connection.profiler_available);
        let registry = connection.core.get_registry_rc().expect("registry");
        let node_id = Rc::new(Cell::new(None));
        let global_id = Rc::new(Cell::new(None));
        let node = node_id.clone();
        let global = global_id.clone();
        let _listener = registry
            .add_listener_local()
            .global(move |object| {
                if object.type_ == pw::types::ObjectType::Profiler {
                    global.set(Some((object.id, object.version)));
                }
                if object.type_ == pw::types::ObjectType::Node
                    && object
                        .props
                        .is_some_and(|props| props.get("node.name") == Some(target.as_str()))
                {
                    node.set(Some(object.id));
                }
            })
            .register();
        let subscription = RefCell::new(None);
        let measurements = Rc::new(Cell::new(0));
        let valid = Rc::new(Cell::new(true));
        let observed_valid = valid.clone();
        let observed = measurements.clone();
        let listener_loop = mainloop.clone();
        let timer = mainloop.loop_().add_timer(move |_| {
            if subscription.borrow().is_some() {
                return;
            }
            if let (Some((id, version)), Some(node_id)) = (global_id.get(), node_id.get()) {
                let observed = observed.clone();
                let loop_ = listener_loop.clone();
                let valid = observed_valid.clone();
                subscription.replace(Some(
                    Profiler::new(
                        registry.clone(),
                        id,
                        version,
                        node_id,
                        move |elapsed_ns, budget_ns| {
                            if elapsed_ns == 0 {
                                return;
                            }
                            valid.set(valid.get() && budget_ns == 2_666_666);
                            observed.set(observed.get() + 1);
                            if observed.get() >= 3 {
                                loop_.quit();
                            }
                        },
                    )
                    .expect("native profiler subscription"),
                ));
            }
        });
        timer
            .update_timer(
                Some(Duration::from_millis(10)),
                Some(Duration::from_millis(10)),
            )
            .into_result()
            .expect("timer");
        let loop_ = mainloop.clone();
        let deadline = mainloop.loop_().add_timer(move |_| loop_.quit());
        deadline
            .update_timer(Some(Duration::from_secs(5)), None)
            .into_result()
            .expect("deadline");
        mainloop.run();
        connection.check().expect("native protocol remains healthy");
        assert!(
            valid.get(),
            "invalid quantum or duration in native measurements"
        );
        assert!(measurements.get() >= 3, "no profiler measurements received");
    }
}
