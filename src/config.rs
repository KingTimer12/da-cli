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
            std::fs::create_dir_all(parent)
                .with_context(|| format!("não consegui criar {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("falha ao serializar config")?;
        std::fs::write(path, text)
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
}
