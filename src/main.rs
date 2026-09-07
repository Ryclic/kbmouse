#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod engine;
mod geometry;
mod gui;
mod instance;
mod labels;
mod platform;
mod runtime;
mod updater;

use anyhow::Result;
use clap::Parser;
use config::Config;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Open hint mode immediately and exit after the interaction
    #[arg(long)]
    hint: bool,
    /// Check for a signed release without starting keyboard capture
    #[arg(long, conflicts_with_all = ["hint", "update"])]
    check_update: bool,
    /// Install the latest signed release and exit (quit running kbmouse first)
    #[arg(long, conflicts_with_all = ["hint", "check_update"])]
    update: bool,
    /// Use a custom configuration file
    #[arg(long)]
    config: Option<PathBuf>,
    /// Enable debug logging
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    if let Err(error) = try_main() {
        eprintln!("kbmouse: {error:#}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let args = Args::parse();
    let filter = if args.verbose {
        "kbmouse=debug"
    } else {
        "kbmouse=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .init();

    let executable = std::env::current_exe()?;
    if args.check_update {
        match updater::check(&executable)? {
            Some(version) => println!("kbmouse {version} is available"),
            None => println!("kbmouse is up to date"),
        }
        return Ok(());
    }
    let instance = instance::SingleInstance::acquire()?;
    if args.update {
        if let Some(version) = updater::check(&executable)? {
            updater::install(&executable, &version, |_, _| {})?;
            println!("Installed kbmouse {version}. Launch kbmouse to use the update.");
        } else {
            println!("kbmouse is up to date");
        }
        return Ok(());
    }
    let config_path = args.config.unwrap_or(Config::path()?);
    let config = Config::load_or_create(&config_path)?;
    tracing::info!(path = %config_path.display(), "loaded configuration");
    #[cfg(target_os = "macos")]
    platform::initialize(args.hint);
    if args.hint {
        let backend = platform::NativeBackend::new(&config)?;
        return runtime::run(backend, config, true, crossbeam_channel::never());
    }

    let runtime_config = config.clone();
    let (config_tx, config_rx) = crossbeam_channel::unbounded();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    let runtime_thread = std::thread::Builder::new()
        .name("kbmouse-runtime".into())
        .spawn(
            move || match platform::NativeBackend::new(&runtime_config) {
                Ok(backend) => {
                    let _ = startup_tx.send(Ok(()));
                    if let Err(error) = runtime::run(backend, runtime_config, false, config_rx) {
                        tracing::error!(%error, "input runtime stopped");
                    }
                }
                Err(error) => {
                    let _ = startup_tx.send(Err(format!("{error:#}")));
                }
            },
        )?;
    startup_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("input runtime stopped during startup"))?
        .map_err(anyhow::Error::msg)?;
    let result = gui::run(config_path, config, config_tx, executable.clone());
    runtime_thread
        .join()
        .map_err(|_| anyhow::anyhow!("input runtime panicked"))?;
    let restart = result?;
    // The runtime has released capture/buttons. Release the socket/mutex before
    // the new process starts so it cannot mistake this process for another instance.
    drop(instance);
    if restart {
        updater::restart(&executable)?;
    }
    Ok(())
}
