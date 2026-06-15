use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_PATH: &str = ".da/config.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    pub build: Build,
    pub deploy: Deploy,
}

fn default_port() -> u16 {
    22
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    Key { key_path: String },
    Password { password: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Build {
    pub command: String,
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Deploy {
    pub remote_dir: String,
    #[serde(default = "default_true")]
    pub clean_remote: bool,
}

fn default_true() -> bool {
    true
}

/// Create a directory tree with owner-only perms (0o700) on Unix.
#[cfg(unix)]
fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.exists() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(dir)
}

#[cfg(not(unix))]
fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Write a file with owner-only perms (0o600) on Unix, chmod'ing even if it
/// already existed with looser perms.
#[cfg(unix)]
fn write_secure(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    f.write_all(data)
}

#[cfg(not(unix))]
fn write_secure(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!("config não encontrada em {} — rode `da init`", path.display())
        })?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("config inválida em {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            create_dir_secure(parent)
                .with_context(|| format!("não consegui criar {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("falha ao serializar config")?;
        // Config may hold a plaintext password — write owner-only (0o600).
        write_secure(path, text.as_bytes())
            .with_context(|| format!("falha ao escrever {}", path.display()))?;
        Ok(())
    }

    pub fn key_path_expanded(&self) -> Option<PathBuf> {
        match &self.auth {
            Auth::Key { key_path } => Some(expand_tilde(key_path)),
            Auth::Password { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(auth: Auth) -> Config {
        Config {
            host: "1.2.3.4".into(),
            port: 22,
            user: "ubuntu".into(),
            auth,
            build: Build {
                command: "npm run build".into(),
                output_dir: "dist".into(),
            },
            deploy: Deploy {
                remote_dir: "/var/www/app".into(),
                clean_remote: true,
            },
        }
    }

    #[test]
    fn roundtrip_key() {
        let c = sample(Auth::Key {
            key_path: "~/.ssh/me.pem".into(),
        });
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn roundtrip_password() {
        let c = sample(Auth::Password {
            password: "hunter2".into(),
        });
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn port_defaults_to_22() {
        let s = r#"
host = "h"
user = "u"
[auth]
type = "password"
password = "p"
[build]
command = "b"
output_dir = "dist"
[deploy]
remote_dir = "/x"
"#;
        let c: Config = toml::from_str(s).unwrap();
        assert_eq!(c.port, 22);
        assert!(c.deploy.clean_remote);
    }

    #[test]
    fn expand_tilde_replaces_home() {
        let home = dirs::home_dir().unwrap();
        let expanded = expand_tilde("~/.ssh/me.pem");
        assert_eq!(expanded, home.join(".ssh/me.pem"));
    }

    #[test]
    fn expand_tilde_passthrough_absolute() {
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_owner_only_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("da_test_perms");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        sample(Auth::Password {
            password: "secret".into(),
        })
        .save(&path)
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config deve ser 0o600, got {mode:o}");
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "dir .da deve ser 0o700, got {dir_mode:o}");
    }
}
