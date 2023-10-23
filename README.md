![a push to talk logo created by dall-e](./pictures/logo-small.png)

# Push to talk - working with both wayland/x11 and pulseaudio (pipewire)

## Usage

At start it will mute all your sources (microphone) and then you will have to press <kbd>Control_Left</kbd>+<kbd>Space</kbd> to unmute.
You can release <kbd>Space</kbd> then, keeping only <kbd>Control_Left</kbd> press to keep unmute.
Releasing <kbd>Control_Left</kbd> will mute again then.

- You can pause/resume the program with sending a `SIGUSR1` signal.
- To set keybind compose of one or two keys, use env var, eg: `env PUSH2TALK_KEYBIND="ControlLeft,KeyO" cargo run` or `env PUSH2TALK_KEYBIND="MetaRight" cargo run`.

## Requirements

User have to be in `input` group (or maybe `plugdev`, depend your distro, check file under `/dev/input/*`).

```bash
sudo usermod -a -G plugdev $USER
sudo usermod -a -G input $USER
```

## Notes

- To debug: `RUST_LOG=debug cargo run -- [--source device]`
