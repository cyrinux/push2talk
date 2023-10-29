use clap::Parser;
use directories_next::BaseDirs;
use fs2::FileExt;
use input::event::keyboard::KeyState::*;
use input::event::keyboard::KeyboardEventTrait;
use input::{Libinput, LibinputInterface};
use itertools::Itertools;
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use log::{debug, info};
use pulsectl::controllers::types::DeviceInfo;
use pulsectl::controllers::{DeviceControl, SourceController};
use signal_hook::flag;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::os::unix::{fs::OpenOptionsExt, io::OwnedFd};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
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

struct MyLibinputInterface;
impl LibinputInterface for MyLibinputInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_RDONLY != 0) | (flags & O_RDWR != 0))
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
    /// List sources devices
    #[arg(short, long)]
    list_devices: bool,
    /// Toggle pause
    #[arg(short, long)]
    toggle_pause: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize cli
    let cli = Cli::parse();

    // List available sources
    if cli.list_devices {
        list_devices()?;
        return Ok(());
    }

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
    let mut libinput_context = Libinput::new_with_udev(MyLibinputInterface);
    libinput_context
        .udev_assign_seat("seat0")
        .expect("Can't connect to libinput on seat0");

    // Create context
    let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

    // Load keymap informations
    let keymap = xkb::Keymap::new_from_names(
        &xkb_context,
        "",   // rules
        "",   // model
        "",   // layout
        "",   // variant
        None, // options
        xkb::COMPILE_NO_FLAGS,
    )
    .expect("Can't init keymap");

    // Parse and validate keybinding environment variable
    let keybind_parsed = parse_keybind()?;
    validate_keybind(&keybind_parsed)?;

    // Parse source environment variable
    let source = parse_source();

    debug!("Settings: source: {source:?}, keybind: {keybind_parsed:?}");

    // Initialize mute state
    let last_mute = Cell::new(true);
    set_sources(true, &source)?;

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

    // Check keybind closure
    let check_keybind = |key: Keysym, pressed: bool| -> bool {
        match key {
            k if Some(k) == second_key => second_key_pressed.set(pressed),
            k if k == first_key => first_key_pressed.set(pressed),
            _ => {}
        }
        !first_key_pressed.get() || second_key.is_some() && !second_key_pressed.get()
    };

    // Main event loop, toggles state based on signals and key events
    let mut is_running = true;

    // Start the application
    info!("Push2talk started");
    loop {
        if sig_pause.swap(false, Ordering::Relaxed) {
            is_running = !is_running;
            info!("Receive SIGUSR1 signal, is running: {is_running}");
        }

        if !is_running {
            thread::sleep(time::Duration::from_secs(1));
            continue;
        }

        libinput_context.dispatch().unwrap();
        for event in libinput_context.by_ref() {
            handle_event(event, &xkb_state, &check_keybind, &last_mute, &source);
        }
    }
}

fn handle_event(
    event: input::Event,
    xkb_state: &xkb::State,
    check_keybind: &dyn Fn(Keysym, bool) -> bool,
    last_mute: &Cell<bool>,
    source: &Option<String>,
) {
    if let input::Event::Keyboard(key_event) = event {
        let keysym = get_keysym(&key_event, xkb_state);
        let name = xkb::keysym_get_name(keysym);
        let pressed = check_pressed(&key_event);
        log::trace!(
            "Key {}: {name}",
            if pressed { "pressed" } else { "released" }
        );
        let should_mute = check_keybind(keysym, pressed);
        if should_mute != last_mute.get() {
            info!("Toggle mute: {}", should_mute);
            last_mute.set(should_mute);
            set_sources(should_mute, source).ok();
        }
    }
}

fn get_keysym(key_event: &input::event::KeyboardEvent, xkb_state: &xkb::State) -> Keysym {
    let keycode = key_event.key() + 8;
    // libinput's keycodes are offset by 8 from XKB keycodes
    xkb_state.key_get_one_sym(keycode.into())
}

fn check_pressed(state: &input::event::KeyboardEvent) -> bool {
    match state.key_state() {
        Released => false,
        Pressed => true,
    }
}

fn list_devices() -> Result<(), Box<dyn Error>> {
    let mut handler = SourceController::create()?;
    let sources = handler.list_devices()?;
    println!("Source devices:");
    sources.iter().for_each(|d| {
        println!("\t* {}", d.description.as_ref().unwrap());
    });
    Ok(())
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
        return Err("Unknown key".into());
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

fn set_sources(mute: bool, source: &Option<String>) -> Result<(), Box<dyn Error>> {
    let mut handler = SourceController::create()?;
    let sources = handler.list_devices()?;

    let devices_to_set = if let Some(src) = source {
        let source = sources
            .iter()
            .filter(|dev| {
                dev.description
                    .as_ref()
                    .map(|desc| desc.contains(src))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<DeviceInfo>>()
            .into_iter()
            .exactly_one()?;

        handler
            .set_default_device(&source.name.clone().unwrap())
            .map_err(|e| format!("Unable to set default device: {e}"))?;

        vec![source]
    } else {
        sources
    };

    devices_to_set
        .iter()
        .for_each(|d| handler.set_device_mute_by_index(d.clone().index, mute));

    Ok(())
}
