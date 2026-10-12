//! PipeWire connection and bounded control-loop round trips.

use crate::Error;
use pipewire::{self as pw, properties::properties};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub(crate) struct Connection {
    pub mainloop: pw::main_loop::MainLoopRc,
    pub core: pw::core::CoreRc,
    pub(crate) failure: Rc<RefCell<Option<Error>>>,
    pub(crate) profiler_available: bool,
    _errors: pw::core::Listener,
}

impl Connection {
    pub fn new() -> Result<Self, Error> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        Self::with_mainloop(mainloop, false)
    }

    pub(crate) fn with_mainloop(
        mainloop: pw::main_loop::MainLoopRc,
        profiling: bool,
    ) -> Result<Self, Error> {
        let context = pw::context::ContextRc::new(
            &mainloop,
            Some(properties! {
                "cpu.zero.denormals" => "true",
            }),
        )?;
        let profiler_available = if profiling {
            // SAFETY: the control-thread context is alive; it owns the loaded protocol extension.
            // Client contexts need this module to marshal profiler events, as in pw-profiler.
            let module = unsafe {
                pw::sys::pw_context_load_module(
                    context.as_raw_ptr(),
                    c"libpipewire-module-profiler".as_ptr(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                )
            };
            if module.is_null() {
                tracing::warn!(
                    "PipeWire profiler extension unavailable; worker overload monitoring remains active"
                );
            }
            !module.is_null()
        } else {
            false
        };
        let core = context
            .connect_rc(None)
            .map_err(|error| Error::Disconnected(error.to_string()))?;
        let failure = Rc::new(RefCell::new(None));
        let errors = failure.clone();
        let loop_ = mainloop.clone();
        let listener = core
            .add_listener_local()
            .error(move |id, _, result, message| {
                // EPIPE/ECONNRESET/ENOTCONN can be reported on an object before the core.
                let error = if id == pw::core::PW_ID_CORE || matches!(result, -32 | -104 | -107) {
                    Error::Disconnected(message.to_owned())
                } else {
                    Error::Message(message.to_owned())
                };
                *errors.borrow_mut() = Some(error);
                loop_.quit();
            })
            .register();
        Ok(Self {
            mainloop,
            core,
            failure,
            profiler_available,
            _errors: listener,
        })
    }

    pub fn check(&self) -> Result<(), Error> {
        if let Some(error) = self.failure.borrow_mut().take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    pub fn roundtrip(&self) -> Result<(), Error> {
        let pending = self.core.sync(0)?;
        let finished = Rc::new(Cell::new(false));
        let done = finished.clone();
        let loop_ = self.mainloop.clone();
        let _listener = self
            .core
            .add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    done.set(true);
                    loop_.quit();
                }
            })
            .register();
        let loop_ = self.mainloop.clone();
        let _timeout = self.mainloop.loop_().add_timer(move |_| loop_.quit());
        _timeout
            .update_timer(Some(Duration::from_secs(5)), None)
            .into_result()
            .map_err(|error| Error::Message(error.to_string()))?;
        self.mainloop.run();
        self.check()?;
        if finished.get() {
            Ok(())
        } else {
            Err(Error::Message("PipeWire request timed out".into()))
        }
    }
}
