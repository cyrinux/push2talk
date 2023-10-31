use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::{Context, FlagSet};
use libpulse_binding::mainloop::threaded::Mainloop;
use log::{error, trace};
use std::error::Error;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::{env, thread};

pub struct Controller {
    source: Option<String>,
}

impl Controller {
    pub fn new() -> Self {
        Controller {
            source: parse_source(),
        }
    }

    pub fn run(&self, rx: Receiver<bool>) -> Result<(), Box<dyn Error>> {
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

        loop {
            if let Ok(mute) = rx.recv() {
                let mut ctx_volume_controller = context.introspect();
                let source = self.source.clone();
                context
                    .introspect()
                    .get_source_info_list(move |devices_list| {
                        if let ListResult::Item(src) = devices_list {
                            let toggle = match &source {
                                Some(v) => src.description.as_ref().map_or(false, |d| v == d),
                                None => true,
                            };
                            trace!("device source: {:?}", src.description);
                            if toggle {
                                ctx_volume_controller
                                    .set_source_mute_by_index(src.index, mute, None);
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
