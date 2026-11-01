//! Updates request dispatch and worker events on the daemon control loop.
use super::{DaemonHost, reply};
use crate::updates::Action;
use macafon_ipc::{ErrorBody, ErrorCode, Event, Outcome, Request, UpdateComponent};
use std::sync::mpsc;
use std::time::Instant;

fn update_error(error: anyhow::Error) -> ErrorBody {
    let message = error.to_string();
    let code = if message.contains("running") {
        ErrorCode::Busy
    } else {
        ErrorCode::BadRequest
    };
    ErrorBody::new(code, message)
}

impl DaemonHost {
    pub(super) fn start_updates(&mut self) {
        if let Err(error) = self.updates.start(
            Action::Startup,
            self.config.sounddeck.downloader_path.clone(),
            None,
        ) {
            tracing::warn!(%error, "Cannot initialize updates");
        }
    }
    fn begin_update(
        &mut self,
        action: Action,
        reply: Option<mpsc::Sender<Outcome>>,
    ) -> Result<(), ErrorBody> {
        self.updates
            .start(action, self.config.sounddeck.downloader_path.clone(), reply)
            .map_err(update_error)?;
        self.hub
            .broadcast(&Event::UpdatesChanged(self.updates.state.clone()));
        Ok(())
    }
    pub(super) fn dispatch_update(&mut self, request: Request, reply: mpsc::Sender<Outcome>) {
        let action = match request {
            Request::UpdatesCheck => {
                // Checks can outlast the IPC deadline. Acknowledge scheduling now; versions,
                // progress and failures are delivered by updates.changed.
                let outcome = match self.begin_update(Action::Check, None) {
                    Ok(()) => Outcome::Result(serde_json::Value::Null),
                    Err(error) => Outcome::Error(error),
                };
                let _ = reply.send(outcome);
                return;
            }
            Request::UpdatesOpenRelease { component } => Action::Open(component),
            _ => return,
        };
        if let Err(error) = self.begin_update(action, Some(reply.clone())) {
            let _ = reply.send(Outcome::Error(error));
        }
    }
    pub(super) fn respond_updates(
        &mut self,
        request: Request,
    ) -> Result<serde_json::Value, ErrorBody> {
        match request {
            Request::UpdatesGet => return reply(&self.updates.state),
            Request::UpdatesSetCheckAutomatically { enabled } => {
                self.config.updates.check_automatically = enabled;
                self.updates.state.check_automatically = enabled;
                self.changed_at = Some(Instant::now());
                self.hub
                    .broadcast(&Event::UpdatesChanged(self.updates.state.clone()));
            }
            Request::UpdatesCancel { component } => self.updates.cancel(component),
            Request::UpdatesInstall { component } | Request::UpdatesRemove { component } => {
                if component != UpdateComponent::App
                    && self
                        .sounddeck
                        .as_ref()
                        .is_some_and(|deck| deck.downloads_active())
                {
                    return Err(ErrorBody::new(
                        ErrorCode::Busy,
                        "a SoundDeck URL download is running",
                    ));
                }
                let action = if matches!(request, Request::UpdatesInstall { .. }) {
                    Action::Install(component)
                } else {
                    Action::Remove(component)
                };
                self.begin_update(action, None)?;
            }
            Request::UpdatesCheck => self.begin_update(Action::Check, None)?,
            Request::UpdatesOpenRelease { component } => {
                self.begin_update(Action::Open(component), None)?
            }
            _ => {
                return Err(ErrorBody::new(
                    ErrorCode::BadRequest,
                    "not an updates request",
                ));
            }
        }
        Ok(serde_json::Value::Null)
    }
    pub(super) fn tick_updates(&mut self) {
        let current = self
            .sounddeck
            .as_ref()
            .map(|deck| deck.downloader().clone());
        if current != self.updates_downloader {
            self.updates_downloader = current;
            self.updates.refresh();
        }
        let outcome = self
            .updates
            .poll(self.config.sounddeck.downloader_path.clone());
        for state in outcome.states {
            self.hub.broadcast(&Event::UpdatesChanged(state));
        }
        if self.config.updates.last_checked_unix != self.updates.state.last_checked_unix {
            self.config.updates.last_checked_unix = self.updates.state.last_checked_unix;
            self.changed_at = Some(Instant::now());
        }
        self.updates_redetect |= outcome.tools_changed;
        if self.updates_redetect
            && let Some(deck) = &mut self.sounddeck
        {
            self.updates_redetect = deck
                .check_downloader(self.config.sounddeck.downloader_path.clone(), None)
                .is_err();
        }
        if outcome.quit {
            let _ = self.respond(Request::DaemonQuit);
        }
        if self.updates.automatic_due()
            && let Err(error) = self.begin_update(Action::Check, None)
        {
            tracing::warn!(%error, "Cannot check for updates");
        }
    }
}
