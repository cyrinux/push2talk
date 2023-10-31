use clap::Parser;
use directories_next::BaseDirs;
use fs2::FileExt;
use log::{error, info, trace};
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet};
use pulse::mainloop::threaded::Mainloop;
use signal_hook::flag;
use std::error::Error;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use std::{
    env,
    sync::{atomic::AtomicBool, Arc},
    thread,
};
extern crate libpulse_binding as pulse;

mod libinput;

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

    // Parse source environment variable
    let source = parse_source();

    // Init channel for set sources
    let (tx_libinput, rx_set_source) = mpsc::channel();

    // Start set source thread
    thread::spawn(move || {
        set_sources(rx_set_source).expect("Error in pulseaudio thread");
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

fn parse_source() -> Option<String> {
    env::var_os("PUSH2TALK_SOURCE").map(|v| v.into_string().unwrap_or_default())
}

fn register_signal(sig_pause: &Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    flag::register(signal_hook::consts::SIGUSR1, Arc::clone(sig_pause))
        .map_err(|e| format!("Unable to register SIGUSR1 signal: {e}"))?;

    Ok(())
}

fn set_sources(rx: Receiver<(bool, Option<String>)>) -> Result<(), Box<dyn Error>> {
    // Create a new standard mainloop
    let mut mainloop = Mainloop::new().ok_or("Failed to create mainloop")?;

    // Create a new context
    let mut context = Context::new(&mainloop, "Push2talk").ok_or("Failed to create new context")?;

    // Connect the context
    context.connect(None, FlagSet::NOFLAGS, None)?;

    // Wait for context to be ready
    mainloop.start()?;
    loop {
        thread::sleep(Duration::from_millis(250));
        if context.get_state() == pulse::context::State::Ready {
            break;
        }

        error!("Waiting for pulseaudio to be ready...");
    }

    // Run the mainloop briefly to process the source info list callback
    loop {
        // Receive block
        if let Ok((mute, source)) = rx.recv() {
            let mut ctx_volume_controller = context.introspect();
            context
                .introspect()
                .get_source_info_list(move |devices_list| {
                    if let ListResult::Item(src) = devices_list {
                        let toggle = match &source {
                            Some(v) => src.description.as_ref().map_or(false, |d| v == d),
                            None => true,
                        };
                        trace!("device source: {:?}", src.description);
                        if toggle {
                            ctx_volume_controller.set_source_mute_by_index(src.index, mute, None);
                        }
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
