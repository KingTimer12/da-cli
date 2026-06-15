use crate::config::{Auth, Config};
use anyhow::{Context, Result, bail};
use std::net::TcpStream;
use std::path::Path;

/// Reject dangerous remote dirs before any delete.
pub fn validate_remote_dir(dir: &str) -> Result<()> {
    let trimmed = dir.trim();
    if trimmed.is_empty() {
        bail!("remote_dir vazio");
    }
    if trimmed == "/" {
        bail!("remote_dir '/' não permitido (apagaria a raiz)");
    }
    Ok(())
}

/// Abstraction over the remote side so the deploy pipeline is testable
/// without a live host.
pub trait Transport {
    /// Delete every entry inside `remote_dir` (not the dir itself).
    fn clean_remote_dir(&mut self, remote_dir: &str) -> Result<()>;
    /// Ensure `remote_dir` exists (mkdir -p semantics).
    fn ensure_dir(&mut self, remote_dir: &str) -> Result<()>;
    /// Upload a single local file to `remote_path`.
    fn upload_file(&mut self, local: &Path, remote_path: &str) -> Result<()>;
}

pub struct Ssh2Transport {
    session: ssh2::Session,
}

impl Ssh2Transport {
    pub fn connect(cfg: &Config) -> Result<Ssh2Transport> {
        let addr = format!("{}:{}", cfg.host, cfg.port);
        let tcp =
            TcpStream::connect(&addr).with_context(|| format!("não consegui conectar em {addr}"))?;
        let mut session = ssh2::Session::new().context("falha ao criar sessão ssh")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("handshake ssh falhou")?;

        match &cfg.auth {
            Auth::Key { .. } => {
                let key = cfg.key_path_expanded().unwrap();
                tracing::debug!("auth por chave: {}", key.display());
                session
                    .userauth_pubkey_file(&cfg.user, None, &key, None)
                    .with_context(|| format!("auth por chave falhou ({})", key.display()))?;
            }
            Auth::Password { password } => {
                tracing::debug!("auth por senha");
                session
                    .userauth_password(&cfg.user, password)
                    .context("auth por senha falhou")?;
            }
        }
        if !session.authenticated() {
            bail!("autenticação ssh falhou");
        }
        Ok(Ssh2Transport { session })
    }

    fn sftp(&self) -> Result<ssh2::Sftp> {
        self.session.sftp().context("falha ao abrir sftp")
    }
}

impl Transport for Ssh2Transport {
    fn clean_remote_dir(&mut self, remote_dir: &str) -> Result<()> {
        validate_remote_dir(remote_dir)?;
        let mut channel = self
            .session
            .channel_session()
            .context("falha ao abrir canal")?;
        let cmd = format!("rm -rf {dir}/* {dir}/.[!.]* 2>/dev/null; true", dir = remote_dir);
        channel.exec(&cmd).context("falha ao executar rm remoto")?;
        let mut out = String::new();
        use std::io::Read;
        channel.read_to_string(&mut out).ok();
        channel.wait_close().ok();
        Ok(())
    }

    fn ensure_dir(&mut self, remote_dir: &str) -> Result<()> {
        let mut channel = self
            .session
            .channel_session()
            .context("falha ao abrir canal")?;
        channel
            .exec(&format!("mkdir -p {remote_dir}"))
            .context("falha ao criar dir remoto")?;
        channel.wait_close().ok();
        Ok(())
    }

    fn upload_file(&mut self, local: &Path, remote_path: &str) -> Result<()> {
        let data =
            std::fs::read(local).with_context(|| format!("falha ao ler {}", local.display()))?;
        let sftp = self.sftp()?;
        if let Some(parent) = Path::new(remote_path).parent() {
            let _ = sftp.mkdir(parent, 0o755);
        }
        use std::io::Write;
        let mut f = sftp
            .create(Path::new(remote_path))
            .with_context(|| format!("falha ao criar remoto {remote_path}"))?;
        f.write_all(&data)
            .with_context(|| format!("falha ao enviar {remote_path}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_root() {
        assert!(validate_remote_dir("/").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_remote_dir("   ").is_err());
    }

    #[test]
    fn accepts_normal_path() {
        assert!(validate_remote_dir("/var/www/app").is_ok());
    }
}
