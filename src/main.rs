use clap::Parser;
use log::{debug, info};
use pulsectl::controllers::{DeviceControl, SourceController};
use rdev::{grab, Event, EventType, Key};
use signal_hook::flag;
use std::{
    cell::Cell,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread, time,
};

// Command line argument parsing
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    source: Option<String>,
}

fn main() {
    // Initialize logging
    setup_logging();

    // Parse command line arguments
    let args = Args::parse();
    let source = Arc::new(args.source);

    // Parse and validate keybinding environment variable
    let keybind_parsed = parse_keybind();
    validate_keybind(&keybind_parsed);

    // Initialize mute state
    let last_mute = Cell::new(true);
    set_sources(true, Arc::clone(&source), &last_mute);

    // Initialize key states
    let first_key = keybind_parsed[0];
    let first_key_pressed = Cell::new(false);
    let second_key = keybind_parsed.get(1).cloned();
    let second_key_pressed = Cell::new(false);

    // Register UNIX signals for pause
    let sig_pause = Arc::new(AtomicBool::new(false));
    register_signal(&sig_pause);

    // Define the callback for key events
    let callback = {
        move |event: Event| -> Option<Event> {
            let check_keybind = |key: Key, pressed: bool| -> bool {
                match key {
                    k if Some(k) == second_key => second_key_pressed.set(pressed),
                    k if k == first_key => first_key_pressed.set(pressed),
                    _ => {}
                }
                match second_key {
                    Some(_) => !(first_key_pressed.get() && second_key_pressed.get()),
                    None => !first_key_pressed.get(),
                }
            };

            let (key, pressed) = match event.event_type {
                EventType::KeyPress(key) => (key, true),
                EventType::KeyRelease(key) => (key, false),
                _ => return Some(event),
            };

            set_sources(check_keybind(key, pressed), Arc::clone(&source), &last_mute);

            Some(event)
        }
    };

    // Pause for a moment before starting the main loop
    thread::sleep(time::Duration::from_secs(1));

    // Start the application
    info!("Push2talk started.");
    main_loop(callback, &sig_pause);
}

fn setup_logging() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );
}

fn parse_keybind() -> Vec<Key> {
    env::var("PUSH2TALK_KEYBIND")
        .unwrap_or("ControlLeft,Space".to_string())
        .split(',')
        .map(|x| x.parse().unwrap_or_else(|_| panic!("Unknown key: {}", x)))
        .collect()
}

fn validate_keybind(keybind: &[Key]) {
    if keybind.is_empty() || keybind.len() > 2 {
        panic!(
            "Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got {}",
            keybind.len()
        );
    }
}

fn register_signal(sig_pause: &Arc<AtomicBool>) {
    flag::register(signal_hook::consts::SIGUSR1, Arc::clone(sig_pause))
        .expect("Unable to register SIGUSR1 signal");
}

fn main_loop(
    callback: impl Fn(Event) -> Option<Event> + 'static + Clone,
    sig_pause: &Arc<AtomicBool>,
) {
    // Main event loop, toggles state based on signals and key events
    let mut is_running = true;
    loop {
        if sig_pause.swap(false, Ordering::Relaxed) {
            is_running = !is_running;
        }

        if !is_running {
            thread::sleep(time::Duration::from_secs(1));
            continue;
        }

        match grab(callback.clone()) {
            Ok(_) => break,
            Err(error) => {
                debug!("Error: {:?}", error);
                thread::sleep(time::Duration::from_secs(1));
            }
        }
    }
}

fn set_sources(mute: bool, source: Arc<Option<String>>, last_mute: &Cell<bool>) {
    // PulseAudio source manipulation logic
    let mut handler = SourceController::create().expect("Can't create pulseaudio handler");
    let sources = handler
        .list_devices()
        .expect("Could not get list of soures devices.");

    if mute != last_mute.get() {
        info!("Toggle mute: {}", mute);
        last_mute.set(mute);
    }

    sources.iter().for_each(|dev| {
        let description = dev.description.as_ref().unwrap().as_str();

        match source.as_ref() {
            Some(src) => {
                // Set default source device if specify
                handler
                    .set_default_device(src)
                    .expect("Unable to set default device");

                // If source specify, toggle only the source that match
                if description.contains(src) {
                    handler.set_device_mute_by_index(dev.index, mute);
                }
            }
            None => {
                // Otherwise, if no specific source set, toggle all source
                handler.set_device_mute_by_index(dev.index, mute);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keybind() {
        // Override the env variable for this test
        std::env::set_var("PUSH2TALK_KEYBIND", "ShiftLeft,ShiftRight");

        let parsed_keys = parse_keybind();
        assert_eq!(parsed_keys, vec![Key::ShiftLeft, Key::ShiftRight]);
    }

    #[test]
    #[should_panic(expected = "Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got 0")]
    fn test_validate_keybind_empty() {
        validate_keybind(&[]);
    }

    #[test]
    #[should_panic(expected = "Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got 3")]
    fn test_validate_keybind_too_many() {
        validate_keybind(&[Key::ShiftLeft, Key::ShiftRight, Key::AltGr]);
    }

    #[test]
    fn test_validate_keybind_single_key() {
        validate_keybind(&[Key::ShiftLeft]);
    }

    #[test]
    fn test_validate_keybind_two_keys() {
        validate_keybind(&[Key::ShiftLeft, Key::ShiftRight]);
    }
}
