use input::event::keyboard::KeyState::*;
use input::event::keyboard::KeyboardEventTrait;
use input::{Libinput, LibinputInterface};
use itertools::Itertools;
use libc::{O_RDWR, O_WRONLY};
use log::{debug, info, trace};
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::{
    fs::OpenOptionsExt,
    io::{AsRawFd, OwnedFd},
};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::{
    cell::Cell,
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use xkbcommon::xkb;
use xkbcommon::xkb::Keysym;

pub struct Controller {
    first_key: Keysym,
    first_key_pressed: Cell<bool>,
    second_key: Option<Keysym>,
    second_key_pressed: Cell<bool>,
    last_mute: Cell<bool>,
    xkb_state: xkb::State,
}

impl Controller {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let keybind_parsed = parse_keybind()?;
        validate_keybind(&keybind_parsed)?;

        let keybind_names = keybind_parsed
            .iter()
            .map(|k| xkb::keysym_get_name(*k))
            .join(",");
        debug!("Using key binding: {keybind_names}");

        Ok(Controller {
            first_key: keybind_parsed[0],
            first_key_pressed: Cell::new(false),
            second_key: keybind_parsed.get(1).cloned(),
            second_key_pressed: Cell::new(false),
            last_mute: Cell::new(false),
            xkb_state: init_xkb_state()?,
        })
    }

    pub fn run(&self, tx: Sender<bool>, sig_pause: Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
        // Mute on init
        tx.send(true)?;

        let mut libinput_context = Libinput::new_with_udev(Push2TalkLibinput);
        libinput_context
            .udev_assign_seat("seat0")
            .map_err(|e| format!("Can't connect to libinput on seat0: {e:?}"))?;

        let mut fds = [libc::pollfd {
            fd: libinput_context.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];

        let poll_timeout = 1000;
        let mut is_running = true;

        loop {
            let poll_err = unsafe { libc::poll(fds.as_mut_ptr(), 1, poll_timeout) } < 0;
            if poll_err {
                // on pause signal send, libc abort polling and
                // receive EINTR error
                if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err("Unable to poll libinput, aborting".into());
            }

            libinput_context.dispatch()?;

            if sig_pause.swap(false, Ordering::Relaxed) {
                is_running = !is_running;
                info!(
                    "Received SIGUSR1 signal, {}",
                    if is_running { "resuming" } else { "pausing" }
                );

                // Toggle mute on pause/resume
                tx.send(is_running)?;

                // ignore final events that happened just before the resume signal
                if is_running {
                    libinput_context.by_ref().for_each(drop);
                }
            }

            for event in libinput_context.by_ref() {
                if is_running {
                    self.handle(event, tx.clone())?;
                }
            }
        }
    }

    fn handle(&self, event: input::Event, tx: Sender<bool>) -> Result<(), Box<dyn Error>> {
        if let input::Event::Keyboard(key_event) = event {
            let keysym = get_keysym(&key_event, &self.xkb_state);
            let pressed = check_pressed(&key_event);
            trace!(
                "Key {}: {}",
                if pressed { "pressed" } else { "released" },
                xkb::keysym_get_name(keysym)
            );

            self.update(keysym, pressed);

            let should_mute = self.should_mute();
            if should_mute != self.last_mute.get() {
                debug!(
                    "Microphone is {}",
                    if should_mute { "muted" } else { "unmuted" }
                );
                self.last_mute.set(should_mute);
                tx.send(should_mute)?;
            }
        };

        Ok(())
    }

    fn update(&self, key: Keysym, pressed: bool) {
        match key {
            k if Some(k) == self.second_key => self.second_key_pressed.set(pressed),
            k if k == self.first_key => self.first_key_pressed.set(pressed),
            _ => {}
        }
    }

    fn should_mute(&self) -> bool {
        !self.first_key_pressed.get() || self.second_key.is_some() && !self.second_key_pressed.get()
    }
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

fn validate_keybind(keybind: &[Keysym]) -> Result<(), Box<dyn Error>> {
    match keybind.len() {
        1 | 2 => Ok(()),
        n => Err(format!("Expected 1 or 2 keys for PUSH2TALK_KEYBIND, got {n}").into()),
    }
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

fn init_xkb_state() -> Result<xkb::State, Box<dyn Error>> {
    let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap =
        xkb::Keymap::new_from_names(&xkb_context, "", "", "", "", None, xkb::COMPILE_NO_FLAGS)
            .ok_or("Unable to initialize xkb keymap")?;

    Ok(xkb::State::new(&keymap))
}

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
        let file = File::from(fd);
        drop(file);
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
}
