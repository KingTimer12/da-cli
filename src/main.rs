mod cli;
mod commands;
mod config;
mod ignore;
mod ssh;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
use std::path::Path;
use std::process::ExitCode;
use tracing::Level;
use tracing_subscriber::fmt::time::ChronoLocal;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    init_logging(verbose);

    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Deploy { dry_run } => run_deploy(dry_run, verbose),
        Commands::Undo { id, dry_run } => run_undo(id, dry_run),
        Commands::Upgrade { force } => commands::upgrade::run(force),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_deploy(dry_run: bool, verbose: bool) -> Result<()> {
    let cfg = Config::load(Path::new(config::CONFIG_PATH))?;
    commands::deploy::run(&cfg, dry_run, verbose)
}

fn run_undo(id: Option<String>, dry_run: bool) -> Result<()> {
    let cfg = Config::load(Path::new(config::CONFIG_PATH))?;
    commands::undo::run(&cfg, id, dry_run)
}

fn init_logging(verbose: bool) {
    let level = if verbose { Level::DEBUG } else { Level::INFO };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_timer(ChronoLocal::new("%H:%M:%S".into()))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}
