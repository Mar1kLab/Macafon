//! Pipe ownership lives on a dedicated thread: a blocked or crashed worker cannot stall audio.
use anyhow::{Context, Result, ensure};
use macafon_stt::protocol::{self, WorkerEvent, WorkerRequest};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

pub enum Input {
    Request(WorkerRequest),
    Audio(Vec<f32>),
}
pub struct Worker {
    pub input: mpsc::SyncSender<Input>,
    pub events: mpsc::Receiver<Result<WorkerEvent, String>>,
    child: Child,
    #[cfg(windows)]
    _job: Job,
}

impl Worker {
    pub fn spawn() -> Result<Self> {
        let mut exe = std::env::current_exe()?;
        exe.set_file_name(if cfg!(windows) {
            "macafon-stt.exe"
        } else {
            "macafon-stt"
        });
        if !exe.exists()
            && exe
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|p| p == "deps")
        {
            exe = exe
                .parent()
                .and_then(|p| p.parent())
                .context("worker path")?
                .join(if cfg!(windows) {
                    "macafon-stt.exe"
                } else {
                    "macafon-stt"
                });
        }
        if !exe.exists() {
            if let Some(target) = exe.parent().and_then(|p| p.parent()) {
                let release = target.join("release").join(if cfg!(windows) {
                    "macafon-stt.exe"
                } else {
                    "macafon-stt"
                });
                if release.exists() {
                    exe = release;
                }
            }
        }
        let mut command = Command::new(exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000 | 0x00004000); // CREATE_NO_WINDOW | BELOW_NORMAL
        }
        let mut child = command.spawn().context("cannot start dictation worker")?;
        #[cfg(windows)]
        let job = match Job::new(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut stdin = child.stdin.take().context("worker stdin")?;
        let mut stdout = child.stdout.take().context("worker stdout")?;
        let (input, receiver) = mpsc::sync_channel(128);
        let (sender, events) = mpsc::channel();
        let failures = sender.clone();
        std::thread::Builder::new()
            .name("dictation-writer".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    let result = match message {
                        Input::Request(request) => serde_json::to_vec(&request)
                            .map_err(std::io::Error::other)
                            .and_then(|payload| {
                                write_frame(&mut stdin, protocol::KIND_JSON, &payload)
                            }),
                        Input::Audio(samples) => {
                            let mut payload = Vec::with_capacity(samples.len() * 4);
                            for sample in samples {
                                payload.extend_from_slice(&sample.to_le_bytes());
                            }
                            write_frame(&mut stdin, protocol::KIND_AUDIO, &payload)
                        }
                    };
                    if result.is_err() {
                        let _ = failures.send(Err("worker pipe closed".into()));
                        break;
                    }
                }
            })?;
        std::thread::Builder::new()
            .name("dictation-reader".into())
            .spawn(move || {
                loop {
                    let event = read_event(&mut stdout)
                        .map_err(|_| "dictation worker closed or sent an invalid frame".into());
                    let failed = event.is_err();
                    if sender.send(event).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            input,
            events,
            child,
            #[cfg(windows)]
            _job: job,
        })
    }
    pub fn send(&self, request: WorkerRequest) -> Result<()> {
        self.input
            .try_send(Input::Request(request))
            .map_err(|_| anyhow::anyhow!("worker is not consuming commands"))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_frame(writer: &mut impl Write, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    writer.write_all(&((payload.len() + 1) as u32).to_le_bytes())?;
    writer.write_all(&[kind])?;
    writer.write_all(payload)?;
    writer.flush()
}
fn read_event(reader: &mut impl Read) -> Result<WorkerEvent> {
    let mut len = [0; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    ensure!(
        (2..=protocol::MAX_FRAME_BYTES).contains(&len),
        "invalid worker frame"
    );
    let mut payload = vec![0; len as usize];
    reader.read_exact(&mut payload)?;
    ensure!(
        payload[0] == protocol::KIND_JSON,
        "unexpected worker frame kind"
    );
    Ok(serde_json::from_slice(&payload[1..])?)
}

#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn new(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::{JobObjects::*, Threading::*};
        // SAFETY: handles and pointers are valid for the synchronous calls; the Job owns its handle.
        unsafe {
            let job = Self(CreateJobObjectW(None, None)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            let process = HANDLE(child.as_raw_handle());
            AssignProcessToJobObject(job.0, process)?;
            let power = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            };
            SetProcessInformation(
                process,
                ProcessPowerThrottling,
                &power as *const _ as _,
                std::mem::size_of_val(&power) as u32,
            )?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the Job handle is owned and closed exactly once.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
