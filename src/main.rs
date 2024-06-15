use clap::Parser;
use directories_next::BaseDirs;
use fs2::FileExt;
use log::{error, info};
use signal_hook::flag;
use std::error::Error;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
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
        Command::new("pkill")
            .args(["-SIGUSR1", "-f", "push2talk"])
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

    let (tx_exit, rx_exit) = mpsc::channel();

    // Register UNIX signals for pause
    let is_paused = Arc::new(Mutex::new(false));
    register_signal(tx_exit.clone(), is_paused.clone())?;

    let (pulseaudio_ctl, tx_libinput) = pulseaudio::Controller::new();

    // Start set source thread
    let is_paused_pulseaudio = is_paused.clone();
    let tx_exit_pulseaudio = tx_exit.clone();
    run_in_thread(tx_exit.clone(), "pulseaudio", move || {
        pulseaudio_ctl.run(tx_exit_pulseaudio, is_paused_pulseaudio)
    })?;

    // Init libinput
    run_in_thread(tx_exit.clone(), "libinput", move || {
        libinput::Controller::new()?.run(tx_libinput, is_paused)
    })?;

    // Start the application
    info!("Push2talk started");

    rx_exit.recv()?;
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

fn run_in_thread<F>(tx_exit: Sender<bool>, name: &str, f: F) -> Result<(), Box<dyn Error>>
where
    F: FnOnce() -> Result<(), Box<dyn Error>> + Send + 'static,
{
    let name = name.to_string();
    thread::Builder::new().name(name.clone()).spawn(move || {
        if let Err(err) = f() {
            error!("Error in thread '{name}': {err:?}");
            if let Err(err) = tx_exit.send(true) {
                error!("Unable to send exit signal from thread '{name}': {err}");
            }
        }
    })?;

    Ok(())
}

fn register_signal(
    tx_exit: Sender<bool>,
    is_paused: Arc<Mutex<bool>>,
) -> Result<(), Box<dyn Error>> {
    let sig_pause = Arc::new(AtomicBool::new(false));

    flag::register(signal_hook::consts::SIGUSR1, Arc::clone(&sig_pause))
        .map_err(|err| format!("Unable to register SIGUSR1 signal: {err}"))?;

    run_in_thread(tx_exit, "signal_catcher", move || loop {
        if !sig_pause.swap(false, Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        let mut lock = is_paused
            .lock()
            .map_err(|err| format!("Deadlock in handling UNIX signal: {err}"))?;

        *lock = !*lock;
        info!(
            "Received SIGUSR1 signal, {}",
            if *lock { "pausing" } else { "resuming" }
        );
    })?;

    Ok(())
}
