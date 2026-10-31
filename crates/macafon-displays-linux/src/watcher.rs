use crate::{Backend, DisplayError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Watcher {
    cancelled: Arc<AtomicBool>,
    stop: Option<mpsc::Sender<()>>,
    bus: Option<zbus::blocking::Connection>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(bus) = self.bus.take() {
            let _ = bus.close();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub fn watch(changed: impl Fn() + Send + 'static) -> Result<Watcher, DisplayError> {
    let backend = crate::backend()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop_flag = cancelled.clone();
    if backend == Backend::Mutter {
        let bus = crate::mutter::connection()?;
        let proxy = zbus::blocking::Proxy::new(
            &bus,
            crate::mutter::DESTINATION,
            crate::mutter::PATH,
            crate::mutter::DESTINATION,
        )?;
        let signals = proxy.receive_signal("MonitorsChanged")?;
        let worker = std::thread::Builder::new()
            .name("macafon-display-watch".into())
            .spawn(move || {
                for _ in signals {
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    changed();
                }
            })?;
        Ok(Watcher {
            cancelled,
            stop: None,
            bus: Some(bus),
            worker: Some(worker),
        })
    } else {
        let (stop, receiver) = mpsc::channel();
        let mut previous = crate::monitors()?;
        let worker = std::thread::Builder::new()
            .name("macafon-display-watch".into())
            .spawn(move || {
                while receiver.recv_timeout(Duration::from_secs(2)).is_err()
                    && !stop_flag.load(Ordering::Acquire)
                {
                    if let Ok(current) = crate::monitors()
                        && current != previous
                    {
                        previous = current;
                        changed();
                    }
                }
            })?;
        Ok(Watcher {
            cancelled,
            stop: Some(stop),
            bus: None,
            worker: Some(worker),
        })
    }
}
