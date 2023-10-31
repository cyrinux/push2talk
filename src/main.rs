use clap::Parser;
use directories_next::BaseDirs;
use fs2::FileExt;
use log::info;
use signal_hook::flag;
use std::error::Error;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self};
use std::{
    sync::{atomic::AtomicBool, Arc},
    thread,
};

mod libinput;
mod pulseaudio;

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

    let libinput_ctl = libinput::Controller::new()?;
    let pulseaudio_ctl = pulseaudio::Controller::new();

    // Parse source environment variable
    let source = pulseaudio::parse_source();

    // Init channel for set sources
    let (tx_libinput, rx_set_source) = mpsc::channel();

    // Start set source thread
    thread::spawn(move || {
        pulseaudio_ctl
            .run(rx_set_source)
            .expect("Error in pulseaudio thread");
        // set_sources(rx_set_source).expect("Error in pulseaudio thread");
    });

    // Register UNIX signals for pause
    let sig_pause = Arc::new(AtomicBool::new(false));
    register_signal(&sig_pause)?;

    // Start the application
    info!("Push2talk started");

    // Init libinput
    libinput_ctl.run(source, tx_libinput, sig_pause)?;

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

fn register_signal(sig_pause: &Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    flag::register(signal_hook::consts::SIGUSR1, Arc::clone(sig_pause))
        .map_err(|e| format!("Unable to register SIGUSR1 signal: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_signal_success() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(register_signal(&flag).is_ok());
    }
}
