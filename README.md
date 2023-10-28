![](https://img.shields.io/crates/d/push2talk)
![](https://img.shields.io/github/issues-raw/cyrinux/push2talk)
![](https://img.shields.io/github/stars/cyrinux/push2talk)
![](https://img.shields.io/aur/version/push2talk-git)
![](https://img.shields.io/crates/v/push2talk)
[![codecov](https://codecov.io/gh/cyrinux/push2talk/branch/main/graph/badge.svg?token=NYY5DRMLM4)](https://codecov.io/gh/cyrinux/push2talk)

![a push to talk logo created by dall-e](./pictures/logo-small.png)

# Push to talk - working with both wayland/x11 and pulseaudio (pipewire)

## 🥅How to use it ?

At start it will mute all your sources (microphone) and then you will have to press <kbd>Control_Left</kbd>+<kbd>Space</kbd> to unmute.
You can release <kbd>Space</kbd><kbd>Control_Left</kbd> then to mute again.

- You can pause/resume the program with sending a `SIGUSR1` signal.

## ⚠️ Requirements

User have to be in `input` group (or maybe `plugdev`, depend your distro, check file under `/dev/input/*`).

```bash
sudo usermod -a -G plugdev $USER
sudo usermod -a -G input $USER
```

## 📦 Installation

- There is a AUR for [archlinux](https://aur.archlinux.org/packages/push2talk-git)
- For other distro, you can `cargo install --git https://github.com/cyrinux/push2talk`

## 🎤 Usage

- To set keybind compose of one or two keys, use env var, eg: `env PUSH2TALK_KEYBIND="ControlLeft,KeyO" push2talk` or `env PUSH2TALK_KEYBIND="MetaRight" push2talk`.

- To get more log: `push2talk [-vvv]`.
- To specify an unique source to manage use the `--source device`.
- There is also a systemd unit provided. `systemctl --user start push2talk.service`
