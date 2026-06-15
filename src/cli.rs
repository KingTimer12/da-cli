use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "da", about = "Deploy Automático", version)]
pub struct Cli {
    /// Saída detalhada (debug)
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Cria .da/config.toml interativamente
    Init,
    /// Roda build e transfere para a VPS
    Deploy {
        /// Mostra o plano sem executar nada destrutivo
        #[arg(long)]
        dry_run: bool,
    },
}
