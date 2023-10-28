use clap::Parser;
use clap_verbosity_flag::Verbosity;
use itertools::Itertools;
use log::{debug, info};
use pulsectl::controllers::types::DeviceInfo;
use pulsectl::controllers::{DeviceControl, SourceController};
use rdev::{grab, Event, EventType, Key};
use signal_hook::flag;
use std::error::Error;
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
    #[command(flatten)]
    verbose: Verbosity,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logging
    setup_logging(args.verbose);

    let source = args.source;

    // Parse and validate keybinding environment variable
    let keybind_parsed = parse_keybind()?;
    validate_keybind(&keybind_parsed)?;

    // Initialize mute state
    let last_mute = Cell::new(true);
    set_sources(true, &source.clone())?;

    // Initialize key states
    let first_key = keybind_parsed[0];
    let first_key_pressed = Cell::new(false);
    let second_key = keybind_parsed.get(1).cloned();
    let second_key_pressed = Cell::new(false);

    // Register UNIX signals for pause
    let sig_pause = Arc::new(AtomicBool::new(false));
    register_signal(&sig_pause);

    // Define the callback for key events
    let callback = move |event: Event| -> Option<Event> {
        let check_keybind = |key: Key, pressed: bool| -> bool {
            match key {
                k if Some(k) == second_key => second_key_pressed.set(pressed),
                k if k == first_key => first_key_pressed.set(pressed),
                _ => {}
            }
            !first_key_pressed.get() || second_key.is_some() && !second_key_pressed.get()
        };

        let (key, pressed) = match event.event_type {
            EventType::KeyPress(key) => (key, true),
            EventType::KeyRelease(key) => (key, false),
            _ => return Some(event),
        };

        let should_mute = check_keybind(key, pressed);
        if should_mute != last_mute.get() {
            info!("Toggle mute: {}", should_mute);
            last_mute.set(should_mute);
            set_sources(should_mute, &source).ok();
        }

        Some(event)
    };

    // Pause for a moment before starting the main loop
    thread::sleep(time::Duration::from_secs(1));

    // Start the application
    info!("Push2talk started.");
    main_loop(callback, &sig_pause);

    Ok(())
}

fn setup_logging(verbose: Verbosity) {
    env_logger::Builder::new()
        .filter_level(verbose.log_level_filter())
        .init();
}

fn parse_keybind() -> Result<Vec<Key>, Box<dyn Error>> {
    env::var("PUSH2TALK_KEYBIND")
        .unwrap_or("ControlLeft,Space".to_string())
        .split(',')
        .map(|x| x.parse().map_err(|_| format!("Unknown key: {}", x).into()))
        .collect()
}

fn validate_keybind(keybind: &[Key]) -> Result<(), Box<dyn Error>> {
    if keybind.is_empty() || keybind.len() > 2 {
        return Err(format!(
            "Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got {}",
            keybind.len()
        )
        .into());
    };

    Ok(())
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
            debug!("Currently in pause, send SIGUSR1 signal to resume");
            thread::sleep(time::Duration::from_secs(1));
            continue;
        }

        if grab(callback.clone()).is_err() {
            thread::sleep(time::Duration::from_secs(1));
        }
    }
}

fn set_sources(mute: bool, source: &Option<String>) -> Result<(), Box<dyn Error>> {
    let mut handler = SourceController::create()?;
    let sources = handler.list_devices()?;

    let devices_to_set = if let Some(src) = source {
        let filtered_devices = sources
            .iter()
            .filter(|dev| {
                dev.description
                    .as_ref()
                    .map(|desc| desc.contains(src))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<DeviceInfo>>();

        vec![filtered_devices.into_iter().exactly_one()?]
    } else {
        sources
    };

    devices_to_set.iter().for_each(|d| {
        let dev = d.clone();
        handler
            .set_default_device(&dev.name.unwrap())
            .expect("Unable to set default device");

        handler.set_device_mute_by_index(dev.index, mute);
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keybind() {
        std::env::set_var("PUSH2TALK_KEYBIND", "ShiftLeft,ShiftRight");
        let parsed_keys = parse_keybind().unwrap();
        assert_eq!(parsed_keys, vec![Key::ShiftLeft, Key::ShiftRight]);
    }

    #[test]
    fn test_validate_keybind_empty() {
        assert!(validate_keybind(&[]).is_err());
    }

    #[test]
    fn test_validate_keybind_too_many() {
        assert!(validate_keybind(&[Key::ShiftLeft, Key::ShiftRight, Key::AltGr]).is_err());
    }

    #[test]
    fn test_validate_keybind_single_key() {
        assert!(validate_keybind(&[Key::ShiftLeft]).is_ok());
    }

    #[test]
    fn test_validate_keybind_two_keys() {
        assert!(validate_keybind(&[Key::ShiftLeft, Key::ShiftRight]).is_ok());
    }
}
