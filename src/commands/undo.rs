//! `da undo` — restaura o backup mais recente (ou `--id`) para o remoto.

use crate::commands::backup;
use crate::config::Config;
use anyhow::{Context, Result};

pub fn run(cfg: &Config, id: Option<String>, dry_run: bool) -> Result<()> {
    let path = backup::resolve(id.as_deref())?;
    let chosen = backup::id_of(&path);
    let target = format!("{}@{}:{}", cfg.user, cfg.host, cfg.port);

    if dry_run {
        tracing::info!("dry-run — nada será alterado");
        tracing::info!("  restauraria backup: {}", path.display());
        tracing::info!("  para:               {}{}", target, cfg.deploy.remote_dir);
        return Ok(());
    }

    let archive = std::fs::read(&path)
        .with_context(|| format!("falha ao ler backup {}", path.display()))?;

    tracing::info!("[1/3] backup escolhido: {} ({})", chosen, crate::commands::deploy::human_size(archive.len()));

    tracing::info!("[2/3] conectando em {target}");
    let mut transport = crate::ssh::Ssh2Transport::connect(cfg)?;

    tracing::info!("[3/3] restaurando para {}", cfg.deploy.remote_dir);
    let start = std::time::Instant::now();
    crate::ssh::Transport::clean_remote_dir(&mut transport, &cfg.deploy.remote_dir)?;
    crate::ssh::Transport::upload_archive(&mut transport, &archive, &cfg.deploy.remote_dir)?;

    tracing::info!(
        "✓ undo concluído em {:.1}s — remoto restaurado ao backup {}",
        start.elapsed().as_secs_f64(),
        chosen
    );
    Ok(())
}
