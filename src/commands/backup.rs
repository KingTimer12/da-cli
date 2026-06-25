//! Backup do conteúdo remoto antes de cada deploy destrutivo, e restauração (undo).
//!
//! Cada deploy que limpa o remoto salva primeiro um `.da/backups/<id>.tar.gz`
//! com tudo que estava lá. `da undo` restaura o backup mais recente (ou `--id`).

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const BACKUP_DIR: &str = ".da/backups";
/// Quantos backups manter; os mais antigos são apagados.
pub const KEEP: usize = 5;

/// Diretório onde os backups são gravados.
pub fn backup_dir() -> PathBuf {
    PathBuf::from(BACKUP_DIR)
}

/// Grava `archive` como um novo backup e poda os antigos. Retorna o caminho.
/// Arquivos vazios (remoto inexistente) não geram backup.
pub fn save(archive: &[u8], id: &str) -> Result<Option<PathBuf>> {
    if archive.is_empty() {
        return Ok(None);
    }
    let dir = backup_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("falha ao criar {}", dir.display()))?;
    let path = dir.join(format!("{id}.tar.gz"));
    std::fs::write(&path, archive)
        .with_context(|| format!("falha ao gravar backup {}", path.display()))?;
    prune(KEEP)?;
    Ok(Some(path))
}

/// Lista backups existentes, do mais recente pro mais antigo (por nome = timestamp).
pub fn list() -> Result<Vec<PathBuf>> {
    let dir = backup_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("falha ao ler {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".tar.gz"))
        .collect();
    // Nomes são timestamps ordenáveis lexicograficamente; mais novo primeiro.
    items.sort();
    items.reverse();
    Ok(items)
}

/// Resolve o backup a restaurar: `id` específico ou o mais recente.
pub fn resolve(id: Option<&str>) -> Result<PathBuf> {
    let all = list()?;
    match id {
        Some(id) => {
            let target = backup_dir().join(format!("{id}.tar.gz"));
            if !target.exists() {
                bail!("backup '{id}' não encontrado em {}", BACKUP_DIR);
            }
            Ok(target)
        }
        None => all
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("nenhum backup em {} — nada pra desfazer", BACKUP_DIR)),
    }
}

/// Mantém só os `keep` backups mais recentes.
fn prune(keep: usize) -> Result<()> {
    let all = list()?;
    for old in all.into_iter().skip(keep) {
        let _ = std::fs::remove_file(&old);
    }
    Ok(())
}

/// Extrai o id (nome sem `.tar.gz`) de um caminho de backup.
pub fn id_of(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.trim_end_matches(".tar.gz").to_string())
        .unwrap_or_default()
}
