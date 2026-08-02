#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod engine;
mod geometry;
mod gui;
mod instance;
mod labels;
mod platform;
mod runtime;

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

    let _instance = instance::SingleInstance::acquire()?;
    let config_path = args.config.unwrap_or(Config::path()?);
    let config = Config::load_or_create(&config_path)?;
    tracing::info!(path = %config_path.display(), "loaded configuration");
    if args.hint {
        let backend = platform::NativeBackend::new(&config)?;
        return runtime::run(backend, config, true, crossbeam_channel::never());
    }

    let runtime_config = config.clone();
    let (config_tx, config_rx) = crossbeam_channel::unbounded();
    let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
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
    gui::run(config_path, config, config_tx)
}
