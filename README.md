![](https://img.shields.io/github/issues-raw/cyrinux/push2talk)
![](https://img.shields.io/github/stars/cyrinux/push2talk)
![](https://img.shields.io/aur/version/push2talk-git)
![](https://img.shields.io/crates/d/push2talk)
![](https://img.shields.io/crates/v/push2talk)

![Push-to-Talk Logo](./pictures/logo-small.png)

# Push-to-Talk: Seamless Integration with Wayland, X11, PulseAudio & PipeWire

## 🥅 Quick Start

Upon initialization, the application mutes all microphones. To unmute, press <kbd>Control_Left</kbd>+<kbd>Space</kbd>, and release to mute again.

- Suspend/resume functionality available via `SIGUSR1`.

## ⚠️ Prerequisites

Membership in the `input` or `plugdev` group may be necessary. Check `/dev/input/*` for your specific distribution.

```bash
sudo usermod -a -G plugdev $USER
sudo usermod -a -G input $USER
```

## 📦 Installation Methods

- Arch Linux users: [AUR package available](https://aur.archlinux.org/packages/push2talk-git)
- Others: Use `cargo install push2talk`

## 🎤 Usage

- Start `push2talk` binary.
- Systemd unit provided: `systemctl --user start push2talk.service`.

## 🎤 Advanced Configuration

- Trace mode for key and source device identification: `env RUST_LOG=trace push2talk`.
- Custom keybinds via environment variables: `env PUSH2TALK_KEYBIND="Control_L,Space" push2talk`.
- Debug logging: `RUST_LOG=debug push2talk`.
- Specify a particular audio source: `env PUSH2TALK_SOURCE="OpenComm by Shokz" push2talk`.

## 😅 Additional Information

- Excludes Easy Effects sources to prevent unintentional "push-to-listen" scenarios.

## 👥 How to Contribute

Contributions are highly welcome.

## 💑 Acknowledgments

Made with love by @cyrinux and @maximbaz.
