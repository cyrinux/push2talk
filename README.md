![](https://img.shields.io/github/issues-raw/cyrinux/push2talk)
![](https://img.shields.io/github/stars/cyrinux/push2talk)
![](https://img.shields.io/aur/version/push2talk-git)

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
- For other distro, you can `cargo install push2talk`

## 🎤 Usage

- To get the code name of the keys you want to use, or the source devices available, start in `trace` mode: `env RUST_LOG=trace push2talk`.
- To set keybind compose of one or two keys, use env var, eg: `env PUSH2TALK_KEYBIND="Control_L,Space" push2talk` or `env PUSH2TALK_KEYBIND="Super_R" push2talk`.
- To get more log: `RUST_LOG=debug push2talk`.
- To specify an unique source to manage, use the env var, eg: `env PUSH2TALK_SOURCE="OpenComm by Shokz" push2talk`.
- There is also a systemd unit provided. `systemctl --user start push2talk.service`

## 👥 Contributing

We welcome contributions!

## 💑 Thanks

Made with love by @cyrinux and @maximbaz.
