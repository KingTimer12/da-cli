use crate::config::{Auth, Config};
use anyhow::{Context, Result, bail};
use std::net::TcpStream;

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
    /// Create `remote_dir` (mkdir -p) and extract a gzip'd tar stream into it
    /// in a single round-trip.
    fn upload_archive(&mut self, archive: &[u8], remote_dir: &str) -> Result<()>;
    /// Pack the *current* contents of `remote_dir` into a gzip'd tar (for backup).
    /// Returns an empty vec if the dir doesn't exist yet.
    fn download_archive(&mut self, remote_dir: &str) -> Result<Vec<u8>>;
    /// List file paths under `remote_dir`, relative to it (posix separators).
    fn list_remote_files(&mut self, remote_dir: &str) -> Result<Vec<String>>;
    /// Delete the given relative paths inside `remote_dir`.
    fn delete_remote_paths(&mut self, remote_dir: &str, rel_paths: &[String]) -> Result<()>;
}

/// Verify the server's host key against `~/.ssh/known_hosts` (TOFU).
///
/// - Match    → proceed.
/// - Mismatch → refuse (possible MITM).
/// - Not found→ show fingerprint, prompt to accept, append to known_hosts.
fn verify_host_key(session: &ssh2::Session, host: &str, port: u16) -> Result<()> {
    use ssh2::{CheckResult, KnownHostFileKind};

    let (key, key_type) = session
        .host_key()
        .ok_or_else(|| anyhow::anyhow!("servidor não enviou host key"))?;

    let mut known = session.known_hosts().context("falha ao abrir known_hosts")?;
    let kh_path = crate::config::expand_tilde("~/.ssh/known_hosts");
    // Missing file is fine — treated as "no known hosts yet".
    let _ = known.read_file(&kh_path, KnownHostFileKind::OpenSSH);

    match known.check_port(host, port, key) {
        CheckResult::Match => Ok(()),
        CheckResult::Mismatch => {
            bail!(
                "HOST KEY MISMATCH para {host}:{port} — possível MITM. \
                 Verifique {} e remova a entrada antiga se a mudança for legítima.",
                kh_path.display()
            )
        }
        CheckResult::Failure => bail!("falha ao verificar host key de {host}:{port}"),
        CheckResult::NotFound => {
            let fp = fingerprint_sha256(session);
            cliclack::note(
                format!("Host {host}:{port} desconhecido"),
                format!("Fingerprint SHA256:\n{fp}"),
            )
            .ok();
            let accept = cliclack::confirm("Confiar nesta máquina e salvar em known_hosts?")
                .initial_value(false)
                .interact()
                .context("não foi possível ler confirmação (TTY ausente?)")?;
            if !accept {
                bail!("host key rejeitado pelo usuário");
            }
            let fmt = host_key_format(key_type)
                .ok_or_else(|| anyhow::anyhow!("tipo de host key não suportado"))?;
            known
                .add(host, key, "added by da", fmt)
                .context("falha ao adicionar host key")?;
            known
                .write_file(&kh_path, KnownHostFileKind::OpenSSH)
                .with_context(|| format!("falha ao gravar {}", kh_path.display()))?;
            tracing::info!("host key de {host}:{port} salvo em {}", kh_path.display());
            Ok(())
        }
    }
}

fn fingerprint_sha256(session: &ssh2::Session) -> String {
    match session.host_key_hash(ssh2::HashType::Sha256) {
        Some(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        None => "<indisponível>".into(),
    }
}

fn host_key_format(t: ssh2::HostKeyType) -> Option<ssh2::KnownHostKeyFormat> {
    use ssh2::{HostKeyType, KnownHostKeyFormat};
    match t {
        HostKeyType::Rsa => Some(KnownHostKeyFormat::SshRsa),
        HostKeyType::Dss => Some(KnownHostKeyFormat::SshDss),
        HostKeyType::Ecdsa256 => Some(KnownHostKeyFormat::Ecdsa256),
        HostKeyType::Ecdsa384 => Some(KnownHostKeyFormat::Ecdsa384),
        HostKeyType::Ecdsa521 => Some(KnownHostKeyFormat::Ecdsa521),
        HostKeyType::Ed25519 => Some(KnownHostKeyFormat::Ed25519),
        HostKeyType::Unknown => None,
    }
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

        verify_host_key(&session, &cfg.host, cfg.port)?;

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

    fn upload_archive(&mut self, archive: &[u8], remote_dir: &str) -> Result<()> {
        use std::io::Write;
        validate_remote_dir(remote_dir)?;
        let mut channel = self
            .session
            .channel_session()
            .context("falha ao abrir canal")?;
        // Create the target and pipe the gzip'd tar straight into `tar -x`.
        let cmd = format!("mkdir -p {dir} && tar -xzf - -C {dir}", dir = remote_dir);
        channel.exec(&cmd).context("falha ao iniciar extração remota")?;

        // Write in chunks so a large archive doesn't block on one giant write.
        for chunk in archive.chunks(64 * 1024) {
            channel.write_all(chunk).context("falha ao enviar arquivo")?;
        }
        channel.send_eof().context("falha ao finalizar envio")?;
        channel.wait_eof().ok();

        // Surface anything tar printed to stderr on failure.
        let mut err = String::new();
        use std::io::Read;
        channel.stderr().read_to_string(&mut err).ok();
        channel.wait_close().ok();

        let code = channel.exit_status().context("sem status de saída remoto")?;
        if code != 0 {
            bail!("extração remota falhou (tar saiu com {code}): {}", err.trim());
        }
        Ok(())
    }

    fn download_archive(&mut self, remote_dir: &str) -> Result<Vec<u8>> {
        use std::io::Read;
        validate_remote_dir(remote_dir)?;
        let mut channel = self
            .session
            .channel_session()
            .context("falha ao abrir canal")?;
        // Empacota o conteúdo atual; se a pasta não existir, sai com tar vazio.
        let cmd = format!(
            "if [ -d {dir} ]; then tar -czf - -C {dir} . ; fi",
            dir = shell_quote(remote_dir)
        );
        channel.exec(&cmd).context("falha ao empacotar remoto")?;
        let mut buf = Vec::new();
        channel
            .read_to_end(&mut buf)
            .context("falha ao baixar backup remoto")?;
        channel.wait_close().ok();
        let code = channel.exit_status().unwrap_or(0);
        if code != 0 {
            bail!("backup remoto falhou (tar saiu com {code})");
        }
        Ok(buf)
    }

    fn list_remote_files(&mut self, remote_dir: &str) -> Result<Vec<String>> {
        use std::io::Read;
        validate_remote_dir(remote_dir)?;
        let mut channel = self
            .session
            .channel_session()
            .context("falha ao abrir canal")?;
        // Caminhos relativos (find imprime "./a/b"); striplado depois.
        let cmd = format!(
            "cd {dir} 2>/dev/null && find . -type f -printf '%P\\n' 2>/dev/null || true",
            dir = shell_quote(remote_dir)
        );
        channel.exec(&cmd).context("falha ao listar remoto")?;
        let mut out = String::new();
        channel.read_to_string(&mut out).ok();
        channel.wait_close().ok();
        let files = out
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        Ok(files)
    }

    fn delete_remote_paths(&mut self, remote_dir: &str, rel_paths: &[String]) -> Result<()> {
        validate_remote_dir(remote_dir)?;
        if rel_paths.is_empty() {
            return Ok(());
        }
        let base = remote_dir.trim_end_matches('/');
        // Apaga em lotes pra não estourar o limite de tamanho de comando.
        for batch in rel_paths.chunks(200) {
            let args: String = batch
                .iter()
                .map(|p| shell_quote(&format!("{base}/{p}")))
                .collect::<Vec<_>>()
                .join(" ");
            let mut channel = self
                .session
                .channel_session()
                .context("falha ao abrir canal")?;
            channel
                .exec(&format!("rm -f {args}"))
                .context("falha ao apagar arquivos remotos")?;
            use std::io::Read;
            let mut sink = String::new();
            channel.read_to_string(&mut sink).ok();
            channel.wait_close().ok();
        }
        Ok(())
    }
}

/// Quote a string for safe use as a single shell argument.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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
