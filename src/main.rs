use clap::Parser;
use directories_next::BaseDirs;
use fs2::FileExt;
use input::event::keyboard::KeyState::*;
use input::event::keyboard::KeyboardEventTrait;
use input::{Libinput, LibinputInterface};
use libc::{O_RDWR, O_WRONLY};
use log::{debug, info};
use signal_hook::flag;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use std::{
    cell::Cell,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread, time,
};
use xkbcommon::xkb;
use xkbcommon::xkb::Keysym;
extern crate libpulse_binding as pulse;

use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet};
use pulse::mainloop::threaded::Mainloop;
use std::sync::mpsc;

struct Push2TalkLibinput;
impl LibinputInterface for Push2TalkLibinput {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read(true)
            .write((flags & O_WRONLY != 0) | (flags & O_RDWR != 0))
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        let _ = File::from(fd);
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Toggle pause
    #[arg(short, long)]
    toggle_pause: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize cli
    let cli = Cli::parse();

    // Send pause signal
    if cli.toggle_pause {
        Command::new("killall")
            .args(["-SIGUSR1", "push2talk"])
            .spawn()
            .expect("Can't pause push2talk");

        println!("Toggle pause.");

        return Ok(());
    }

    // Ensure that only one instance run
    let lock_file = take_lock()?;
    if lock_file.try_lock_exclusive().is_err() {
        return Err("Another instance is already running.".into());
    }

    // Initialize logging
    setup_logging();

    // Init libinput
    let mut libinput_context = Libinput::new_with_udev(Push2TalkLibinput);
    libinput_context
        .udev_assign_seat("seat0")
        .map_err(|e| format!("Can't connect to libinput on seat0: {e:?}"))?;

    // Create context
    let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    // Load keymap informations
    let keymap =
        xkb::Keymap::new_from_names(&xkb_context, "", "", "", "", None, xkb::COMPILE_NO_FLAGS)
            .unwrap();

    // Parse and validate keybinding environment variable
    let keybind_parsed = parse_keybind()?;
    validate_keybind(&keybind_parsed)?;

    // Parse source environment variable
    let source = parse_source();

    debug!("Settings: source: {source:?}, keybind: {keybind_parsed:?}");

    // Initialize mute state
    let last_mute = Cell::new(true);

    // Init tx/rx
    let (tx, rx): (
        Sender<(bool, Option<String>)>,
        Receiver<(bool, Option<String>)>,
    ) = mpsc::channel();

    // Start set source thread
    let tx_set_source = tx.clone();
    thread::spawn(move || {
        set_sources(rx);
    });

    // Mute on init
    let _ = tx_set_source.send((true, source.clone()));

    // Initialize key states
    let first_key = keybind_parsed[0];
    let first_key_pressed = Cell::new(false);
    let second_key = keybind_parsed.get(1).cloned();
    let second_key_pressed = Cell::new(false);

    // Register UNIX signals for pause
    let sig_pause = Arc::new(AtomicBool::new(false));
    register_signal(&sig_pause)?;

    // Create the state tracker
    let xkb_state = xkb::State::new(&keymap);

    // Main event loop, toggles state based on signals and key events
    let mut is_running = true;

    // Check keybind closure
    let check_keybind = |key: Keysym, pressed: bool| -> bool {
        match key {
            k if Some(k) == second_key => second_key_pressed.set(pressed),
            k if k == first_key => first_key_pressed.set(pressed),
            _ => {}
        }
        !first_key_pressed.get() || second_key.is_some() && !second_key_pressed.get()
    };

    // Start the application
    info!("Push2talk started");
    loop {
        if sig_pause.swap(false, Ordering::Relaxed) {
            is_running = !is_running;
            info!(
                "Receive SIGUSR1 signal, {}",
                if is_running { "resuming" } else { "pausing" }
            )
        }

        if !is_running {
            thread::sleep(time::Duration::from_secs(1));
            continue;
        }

        libinput_context.dispatch()?;
        for event in libinput_context.by_ref() {
            event_handler(
                &xkb_state,
                check_keybind,
                &last_mute,
                event,
                &source,
                tx.clone(),
            )?;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn event_handler(
    xkb_state: &xkb::State,
    check_keybind: impl Fn(Keysym, bool) -> bool,
    last_mute: &Cell<bool>,
    event: input::Event,
    source: &Option<String>,
    tx: Sender<(bool, Option<String>)>,
) -> Result<(), Box<dyn Error>> {
    let xkb_state = xkb_state;
    let check_keybind: &dyn Fn(Keysym, bool) -> bool = &check_keybind;
    let last_mute = last_mute;
    if let input::Event::Keyboard(key_event) = event {
        let keysym = get_keysym(&key_event, xkb_state);
        let pressed = check_pressed(&key_event);
        log::trace!(
            "Key {}: {}",
            if pressed { "pressed" } else { "released" },
            xkb::keysym_get_name(keysym)
        );
        let should_mute = check_keybind(keysym, pressed);
        if should_mute != last_mute.get() {
            info!("Toggle {}", if should_mute { "mute" } else { "unmute" });
            last_mute.set(should_mute);
            let _ = tx.send((should_mute, source.clone()));
        }
    };

    Ok(())
}

fn get_keysym(key_event: &input::event::KeyboardEvent, xkb_state: &xkb::State) -> Keysym {
    // libinput's keycodes are offset by 8 from XKB keycodes
    let keycode = key_event.key() + 8;
    xkb_state.key_get_one_sym(keycode.into())
}

fn check_pressed(state: &input::event::KeyboardEvent) -> bool {
    match state.key_state() {
        Released => false,
        Pressed => true,
    }
}

fn take_lock() -> Result<std::fs::File, Box<dyn Error>> {
    let base_dirs = BaseDirs::new().ok_or("Cannot find base directories")?;
    let mut lock_path = PathBuf::from(
        base_dirs
            .runtime_dir()
            .ok_or("Cannot find XDG runtime directory")?,
    );
    lock_path.push("push2talk.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(lock_path)?;
    Ok(lock_file)
}

fn setup_logging() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );
}

fn parse_keybind() -> Result<Vec<Keysym>, Box<dyn Error>> {
    let keybind = env::var("PUSH2TALK_KEYBIND")
        .unwrap_or("Control_L,Space".to_string())
        .split(',')
        .map(|k| xkb::keysym_from_name(k, xkb::KEYSYM_CASE_INSENSITIVE))
        .collect::<Vec<Keysym>>();

    if keybind
        .iter()
        .any(|k| *k == xkb::keysym_from_name("KEY_NoSymbol", xkb::KEYSYM_CASE_INSENSITIVE))
    {
        return Err("Unable to parse keybind".into());
    }

    Ok(keybind)
}

fn parse_source() -> Option<String> {
    env::var_os("PUSH2TALK_SOURCE").map(|v| v.into_string().unwrap_or_default())
}

fn validate_keybind(keybind: &[Keysym]) -> Result<(), Box<dyn Error>> {
    match keybind.len() {
        1 | 2 => Ok(()),
        n => Err(format!("Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got {n}").into()),
    }
}

fn register_signal(sig_pause: &Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    flag::register(signal_hook::consts::SIGUSR1, Arc::clone(sig_pause))
        .map_err(|e| format!("Unable to register SIGUSR1 signal: {e}"))?;

    Ok(())
}

fn set_sources(rx: Receiver<(bool, Option<String>)>) {
    // Create a new standard mainloop
    let mut mainloop = Mainloop::new().expect("Failed to create mainloop");

    // Create a new context
    let mut lister =
        Context::new(&mainloop, "ToggleMuteSources").expect("Failed to create new context");

    // Connect the context
    lister
        .connect(None, FlagSet::NOFLAGS, None)
        .expect("Failed to connect context");

    // Wait for context to be ready
    mainloop.start().expect("Start mute loop");

    loop {
        mainloop.wait();
        if lister.get_state() == pulse::context::State::Ready {
            break;
        }
    }

    // Run the mainloop briefly to process the source info list callback
    loop {
        match rx.recv() {
            Ok((mute, source)) => {
                mainloop.wait();
                let mut muter = lister.introspect();
                lister
                    .introspect()
                    .get_source_info_list(move |devices_list| match devices_list {
                        ListResult::Item(src) => {
                            let desc = src.description.clone().unwrap();
                            log::trace!("source: {:?}", desc);
                            let toggle = match &source {
                                Some(v) => v == &desc,
                                None => true,
                            };
                            if toggle {
                                muter.set_source_mute_by_index(src.index, mute, None);
                            }
                        }
                        _ => {}
                    });
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_keybind_default() {
        // Assuming default keybinds are Control_L and Space
        std::env::remove_var("PUSH2TALK_KEYBIND");
        let keybind = parse_keybind().unwrap();
        assert_eq!(keybind.len(), 2);
        // Assuming default keybinds are Control_L and Space
        assert_eq!(
            keybind[0],
            xkb::keysym_from_name("Control_L", xkb::KEYSYM_CASE_INSENSITIVE)
        );
        assert_eq!(
            keybind[1],
            xkb::keysym_from_name("Space", xkb::KEYSYM_CASE_INSENSITIVE)
        );
    }

    #[test]
    fn test_parse_keybind_with_2_valid_keys() {
        std::env::set_var("PUSH2TALK_KEYBIND", "Control_L,O");
        let keybind = parse_keybind().unwrap();
        assert_eq!(keybind.len(), 2);
        assert_eq!(
            keybind[0],
            xkb::keysym_from_name("Control_L", xkb::KEYSYM_CASE_INSENSITIVE)
        );
        assert_eq!(
            keybind[1],
            xkb::keysym_from_name("O", xkb::KEYSYM_CASE_INSENSITIVE)
        );
        std::env::remove_var("PUSH2TALK_KEYBIND");
    }

    #[test]
    fn test_parse_keybind_with_invalid_key() {
        std::env::set_var("PUSH2TALK_KEYBIND", "InvalidKey");
        assert!(parse_keybind().is_err());
        std::env::remove_var("PUSH2TALK_KEYBIND");
    }

    #[test]
    fn test_validate_keybind_with_2_keys_is_valid() {
        let keybind = vec![
            xkb::keysym_from_name("Control_L", xkb::KEYSYM_CASE_INSENSITIVE),
            xkb::keysym_from_name("Space", xkb::KEYSYM_CASE_INSENSITIVE),
        ];
        assert!(validate_keybind(&keybind).is_ok());
    }

    #[test]
    fn test_validate_keybind_with_3_keys_is_invalid() {
        let keybind = vec![
            xkb::keysym_from_name("Control_L", xkb::KEYSYM_CASE_INSENSITIVE),
            xkb::keysym_from_name("Space", xkb::KEYSYM_CASE_INSENSITIVE),
            xkb::keysym_from_name("Shift_R", xkb::KEYSYM_CASE_INSENSITIVE),
        ];
        assert!(validate_keybind(&keybind).is_err());
    }

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

    #[test]
    fn test_register_signal_success() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(register_signal(&flag).is_ok());
    }
}
