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
    /// Restaura o backup do remoto feito antes do último deploy
    Undo {
        /// Id do backup (ex: 20260625-143000). Padrão: o mais recente
        #[arg(long)]
        id: Option<String>,
        /// Mostra o que seria restaurado sem alterar nada
        #[arg(long)]
        dry_run: bool,
    },
    /// Baixa e instala a última versão do GitHub
    Upgrade {
        /// Reinstala mesmo se já estiver na última versão
        #[arg(long)]
        force: bool,
    },
}
