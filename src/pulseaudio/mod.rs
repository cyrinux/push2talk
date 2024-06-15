use libpulse_binding::callbacks::ListResult;

use libpulse_binding::context::{
    subscribe::Facility, subscribe::InterestMaskSet, subscribe::Operation, Context, FlagSet,
};
use libpulse_binding::mainloop::threaded::Mainloop;

use log::{error, trace};
use std::error::Error;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, thread};

pub struct Controller {
    source: Option<String>,
    tx: Sender<bool>,
    rx: Receiver<bool>,
}

impl Controller {
    pub fn new() -> (Self, Sender<bool>) {
        let (tx, rx) = mpsc::channel();

        (
            Controller {
                source: parse_source(),
                tx: tx.clone(),
                rx,
            },
            tx,
        )
    }

    pub fn run(
        &self,
        tx_exit: Sender<bool>,
        is_paused: Arc<Mutex<bool>>,
    ) -> Result<(), Box<dyn Error>> {
        let mut mainloop = Mainloop::new().ok_or("Failed to create mainloop")?;

        let mut context =
            Context::new(&mainloop, "Push2talk").ok_or("Failed to create new context")?;

        context.connect(None, FlagSet::NOFLAGS, None)?;

        // Wait for context to be ready
        mainloop.start()?;
        loop {
            thread::sleep(Duration::from_millis(250));
            if context.get_state() == libpulse_binding::context::State::Ready {
                break;
            }

            error!("Waiting for pulseaudio to be ready...");
        }

        // Subscribe to card changes
        context.subscribe(InterestMaskSet::CARD, |_| {});

        // Set the subscribe callback to mute devices on cards change
        // or new/remove devices
        let tx = self.tx.clone();
        context.set_subscribe_callback(Some(Box::new(move |facility, operation, _index| {
            match (is_paused.lock(), facility, operation) {
                (Err(err), _, _) => {
                    error!("Deadlock in pulseaudio checking if we are paused: {err:?}");
                    if let Err(err) = tx_exit.send(true) {
                        error!("Unable to send exit signal from pulseaudio callback: {err:?}");
                    }
                }
                (Ok(is_paused), _, _) if *is_paused => (),
                (_, Some(Facility::Card), Some(Operation::Changed))
                | (_, Some(Facility::Card), Some(Operation::Removed))
                | (_, Some(Facility::Card), Some(Operation::New)) => {
                    trace!("Card changed, added or removed device => muting");
                    if let Err(err) = tx.send(true) {
                        error!("Can't mute devices, ignoring...: {err}");
                    };
                }
                _ => (),
            }
        })));

        loop {
            if let Ok(mute) = self.rx.recv() {
                let mut ctx_volume_controller = context.introspect();
                let source = self.source.clone();
                context
                    .introspect()
                    .get_source_info_list(move |devices_list| {
                        if let ListResult::Item(src) = devices_list {
                            let toggle = match &source {
                                Some(v) => src.description.as_ref().map_or(false, |d| v == d),
                                None => src
                                    .description
                                    .as_ref()
                                    .map_or(true, |d| !d.to_lowercase().contains("easy effects")),
                            };
                            trace!("device source: {:?}", src.description);
                            if toggle {
                                ctx_volume_controller.set_source_mute_by_index(
                                    src.index,
                                    src.active_port.is_some() && mute,
                                    None,
                                );
                            }
                        }
                    });
            }
        }
    }
}

fn parse_source() -> Option<String> {
    env::var_os("PUSH2TALK_SOURCE").map(|v| v.into_string().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_valid() {
        std::env::set_var("PUSH2TALK_SOURCE", "SourceName");
        assert_eq!(parse_source(), Some("SourceName".to_string()));
        std::env::remove_var("PUSH2TALK_SOURCE");
    }

    #[test]
    fn test_parse_source_empty() {
        std::env::remove_var("PUSH2TALK_SOURCE");
        assert_eq!(parse_source(), None);
    }
}
