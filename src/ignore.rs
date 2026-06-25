//! `.daignore` support (gitignore semantics).
//!
//! Three optional files in the project root:
//! - `.daignore`         — geral: vale pro client (não envia) e pro server (não apaga).
//! - `.daignore.client`  — só client: não inclui esses arquivos no pacote enviado.
//! - `.daignore.server`  — só server: não apaga esses arquivos no remoto.
//!
//! General patterns são aplicados aos dois conjuntos.

use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

pub const GENERAL: &str = ".daignore";
pub const CLIENT: &str = ".daignore.client";
pub const SERVER: &str = ".daignore.server";

/// Matchers de ignore resolvidos a partir dos arquivos `.daignore*`.
pub struct DaIgnore {
    client: Gitignore,
    server: Gitignore,
    /// Quantos arquivos `.daignore*` foram efetivamente carregados.
    pub loaded: Vec<&'static str>,
}

fn build(root: &Path, files: &[&str]) -> Result<Gitignore> {
    let mut b = GitignoreBuilder::new(root);
    for f in files {
        let p = root.join(f);
        if p.exists() {
            if let Some(err) = b.add(&p) {
                return Err(err).with_context(|| format!("falha ao ler {f}"));
            }
        }
    }
    b.build().context("falha ao compilar padrões .daignore")
}

impl DaIgnore {
    /// Carrega os `.daignore*` de `root`. Ausência de arquivos = nada ignorado.
    pub fn load(root: &Path) -> Result<DaIgnore> {
        let mut loaded = Vec::new();
        for (name, f) in [(GENERAL, GENERAL), (CLIENT, CLIENT), (SERVER, SERVER)] {
            let _ = name;
            if root.join(f).exists() {
                loaded.push(f);
            }
        }
        Ok(DaIgnore {
            client: build(root, &[GENERAL, CLIENT])?,
            server: build(root, &[GENERAL, SERVER])?,
            loaded,
        })
    }

    /// True se o caminho relativo deve ser EXCLUÍDO do pacote (client).
    pub fn client_excludes(&self, rel: &str) -> bool {
        self.client
            .matched_path_or_any_parents(rel, false)
            .is_ignore()
    }

    /// True se o caminho relativo deve ser PRESERVADO no remoto (server).
    pub fn server_protects(&self, rel: &str) -> bool {
        self.server
            .matched_path_or_any_parents(rel, false)
            .is_ignore()
    }

    /// Há ao menos um padrão de proteção do server?
    pub fn has_server_rules(&self) -> bool {
        self.server.num_ignores() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(files: &[(&str, &str)]) -> std::path::PathBuf {
        // Nome único por conjunto de arquivos pra evitar colisão entre testes paralelos.
        let tag: String = files.iter().map(|(n, _)| *n).collect::<Vec<_>>().join("_");
        let dir = std::env::temp_dir().join(format!("da_ign_{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn general_applies_to_both() {
        let dir = setup(&[(GENERAL, "*.map\n")]);
        let ig = DaIgnore::load(&dir).unwrap();
        assert!(ig.client_excludes("app.js.map"));
        assert!(ig.server_protects("app.js.map"));
        assert!(!ig.client_excludes("app.js"));
    }

    #[test]
    fn client_only_does_not_protect_server() {
        let dir = setup(&[(CLIENT, "secret.env\n")]);
        let ig = DaIgnore::load(&dir).unwrap();
        assert!(ig.client_excludes("secret.env"));
        assert!(!ig.server_protects("secret.env"));
    }

    #[test]
    fn server_only_does_not_exclude_client() {
        let dir = setup(&[(SERVER, "uploads/\n")]);
        let ig = DaIgnore::load(&dir).unwrap();
        assert!(ig.server_protects("uploads/img.png"));
        assert!(!ig.client_excludes("uploads/img.png"));
        assert!(ig.has_server_rules());
    }

    #[test]
    fn missing_files_ignore_nothing() {
        let dir = setup(&[]);
        let ig = DaIgnore::load(&dir).unwrap();
        assert!(!ig.client_excludes("anything"));
        assert!(!ig.server_protects("anything"));
        assert!(!ig.has_server_rules());
    }
}
