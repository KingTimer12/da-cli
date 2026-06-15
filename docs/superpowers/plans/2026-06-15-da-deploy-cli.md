# DA Deploy CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust CLI `da` with `init` and `deploy` commands that automate frontend deploy to a VPS over SSH/SFTP.

**Architecture:** Synchronous pipeline. `config` owns TOML persistence + validation, `ssh` owns transport behind a `Transport` trait (real impl uses `ssh2`; tests use a fake), `commands/*` orchestrate, `cli` parses. Logging via `tracing` to stderr; stdout stays clean.

**Tech Stack:** clap, serde, toml, ssh2 (vendored-openssl), anyhow, dialoguer, indicatif, tracing, tracing-subscriber, walkdir, dirs.

---

## File Structure

```
Cargo.toml         # deps
src/
  main.rs          # init tracing, clap parse, dispatch, exit codes
  cli.rs           # Cli + Commands (clap derive), -v/--verbose
  config.rs        # Config, Auth enum, load/save, ~ expansion, validation
  ssh.rs           # Transport trait, Ssh2Transport, remote_dir safety
  commands/
    mod.rs
    init.rs        # interactive prompts -> .da/config.toml + .gitignore
    deploy.rs      # pipeline + --dry-run, generic over Transport
```

---

## Task 1: Cargo deps + module skeleton

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Create: `src/cli.rs`, `src/config.rs`, `src/ssh.rs`, `src/commands/mod.rs`, `src/commands/init.rs`, `src/commands/deploy.rs`

- [ ] **Step 1: Add dependencies to `Cargo.toml`**

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
ssh2 = { version = "0.9", features = ["vendored-openssl"] }
anyhow = "1"
dialoguer = "0.11"
indicatif = "0.17"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
walkdir = "2"
dirs = "5"
```

- [ ] **Step 2: Create empty module files with minimal valid content**

`src/commands/mod.rs`:
```rust
pub mod deploy;
pub mod init;
```

`src/cli.rs`, `src/config.rs`, `src/ssh.rs`, `src/commands/init.rs`, `src/commands/deploy.rs`: each just `// placeholder` for now (filled by later tasks).

- [ ] **Step 3: Wire modules in `src/main.rs`**

```rust
mod cli;
mod commands;
mod config;
mod ssh;

fn main() {
    println!("placeholder");
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: builds (warnings about unused modules OK).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "chore: add deps and module skeleton"
```

---

## Task 2: Config model + persistence (TDD)

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write failing tests**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(auth: Auth) -> Config {
        Config {
            host: "1.2.3.4".into(),
            port: 22,
            user: "ubuntu".into(),
            auth,
            build: Build { command: "npm run build".into(), output_dir: "dist".into() },
            deploy: Deploy { remote_dir: "/var/www/app".into(), clean_remote: true },
        }
    }

    #[test]
    fn roundtrip_key() {
        let c = sample(Auth::Key { key_path: "~/.ssh/me.pem".into() });
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn roundtrip_password() {
        let c = sample(Auth::Password { password: "hunter2".into() });
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib config`
Expected: FAIL — `expand_tilde` not defined, load/save not defined.

- [ ] **Step 3: Implement persistence + helpers**

Add below the structs in `src/config.rs`:
```rust
use anyhow::{Context, Result};

pub const CONFIG_PATH: &str = ".da/config.toml";

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
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("config não encontrada em {} — rode `da init`", path.display()))?;
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
        std::fs::write(path, text).with_context(|| format!("falha ao escrever {}", path.display()))?;
        Ok(())
    }

    pub fn key_path_expanded(&self) -> Option<PathBuf> {
        match &self.auth {
            Auth::Key { key_path } => Some(expand_tilde(key_path)),
            Auth::Password { .. } => None,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib config`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: config model with toml persistence and tilde expansion"
```

---

## Task 3: Remote-dir safety validation (TDD)

**Files:**
- Modify: `src/ssh.rs`

- [ ] **Step 1: Write failing tests**

```rust
use anyhow::{bail, Result};

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
```

- [ ] **Step 2: Run tests to verify they pass** (function written inline with test)

Run: `cargo test --lib ssh`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**

```bash
git add src/ssh.rs
git commit -m "feat: remote_dir safety validation"
```

---

## Task 4: Transport trait + ssh2 implementation

**Files:**
- Modify: `src/ssh.rs`

- [ ] **Step 1: Define the `Transport` trait**

Add to `src/ssh.rs` (above the tests):
```rust
use crate::config::{Auth, Config};
use std::net::TcpStream;
use std::path::Path;

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
```

- [ ] **Step 2: Implement `Ssh2Transport`**

```rust
pub struct Ssh2Transport {
    session: ssh2::Session,
}

impl Ssh2Transport {
    pub fn connect(cfg: &Config) -> Result<Ssh2Transport> {
        let addr = format!("{}:{}", cfg.host, cfg.port);
        let tcp = TcpStream::connect(&addr)
            .with_context(|| format!("não consegui conectar em {addr}"))?;
        let mut session = ssh2::Session::new().context("falha ao criar sessão ssh")?;
        session.set_tcp_stream(tcp);
        session.handshake().context("handshake ssh falhou")?;

        match &cfg.auth {
            Auth::Key { .. } => {
                let key = cfg.key_path_expanded().unwrap();
                session
                    .userauth_pubkey_file(&cfg.user, None, &key, None)
                    .with_context(|| format!("auth por chave falhou ({})", key.display()))?;
            }
            Auth::Password { password } => {
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
        // Use a remote shell rm for recursive delete of contents.
        let mut channel = self.session.channel_session().context("falha ao abrir canal")?;
        let cmd = format!("rm -rf {}/* {}/.[!.]* 2>/dev/null; true", remote_dir, remote_dir);
        channel.exec(&cmd).context("falha ao executar rm remoto")?;
        let mut out = String::new();
        use std::io::Read;
        channel.read_to_string(&mut out).ok();
        channel.wait_close().ok();
        Ok(())
    }

    fn ensure_dir(&mut self, remote_dir: &str) -> Result<()> {
        let mut channel = self.session.channel_session().context("falha ao abrir canal")?;
        channel
            .exec(&format!("mkdir -p {}", remote_dir))
            .context("falha ao criar dir remoto")?;
        channel.wait_close().ok();
        Ok(())
    }

    fn upload_file(&mut self, local: &Path, remote_path: &str) -> Result<()> {
        let data = std::fs::read(local)
            .with_context(|| format!("falha ao ler {}", local.display()))?;
        let sftp = self.sftp()?;
        // ensure parent dir on remote
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds. (`vendored-openssl` compiles libssh2+openssl on first build — may be slow.)

- [ ] **Step 4: Commit**

```bash
git add src/ssh.rs
git commit -m "feat: Transport trait and ssh2 implementation"
```

---

## Task 5: Deploy pipeline + dry-run (TDD with fake transport)

**Files:**
- Modify: `src/commands/deploy.rs`

- [ ] **Step 1: Write the upload-plan helper + failing test**

`src/commands/deploy.rs`:
```rust
use crate::config::Config;
use crate::ssh::{validate_remote_dir, Transport};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// One file to upload: local source + its remote destination path.
#[derive(Debug, PartialEq)]
pub struct UploadItem {
    pub local: PathBuf,
    pub remote: String,
}

/// Walk `output_dir` and compute remote paths under `remote_dir`,
/// preserving the relative tree. Files only (dirs created on upload).
pub fn build_upload_plan(output_dir: &Path, remote_dir: &str) -> Result<Vec<UploadItem>> {
    if !output_dir.exists() {
        bail!("output_dir não existe: {}", output_dir.display());
    }
    let mut items = Vec::new();
    for entry in WalkDir::new(output_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(output_dir).unwrap();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            items.push(UploadItem {
                local: entry.path().to_path_buf(),
                remote: format!("{}/{}", remote_dir.trim_end_matches('/'), rel_str),
            });
        }
    }
    items.sort_by(|a, b| a.remote.cmp(&b.remote));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn plan_maps_nested_files_to_remote() {
        let tmp = std::env::temp_dir().join("da_test_plan");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("index.html"), "x").unwrap();
        std::fs::write(tmp.join("sub/app.js"), "y").unwrap();

        let plan = build_upload_plan(&tmp, "/var/www/app").unwrap();
        let remotes: Vec<_> = plan.iter().map(|i| i.remote.as_str()).collect();
        assert_eq!(remotes, vec!["/var/www/app/index.html", "/var/www/app/sub/app.js"]);
    }

    #[test]
    fn plan_errors_when_missing() {
        let missing = std::env::temp_dir().join("da_test_missing_xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(build_upload_plan(&missing, "/x").is_err());
    }

    // Fake transport records calls; no network.
    #[derive(Default)]
    struct FakeTransport {
        cleaned: RefCell<Vec<String>>,
        uploaded: RefCell<Vec<String>>,
    }
    impl Transport for FakeTransport {
        fn clean_remote_dir(&mut self, d: &str) -> Result<()> {
            self.cleaned.borrow_mut().push(d.to_string());
            Ok(())
        }
        fn ensure_dir(&mut self, _d: &str) -> Result<()> {
            Ok(())
        }
        fn upload_file(&mut self, _l: &Path, r: &str) -> Result<()> {
            self.uploaded.borrow_mut().push(r.to_string());
            Ok(())
        }
    }

    #[test]
    fn transfer_uploads_all_and_cleans() {
        let tmp = std::env::temp_dir().join("da_test_transfer");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("a.txt"), "a").unwrap();

        let plan = build_upload_plan(&tmp, "/srv/app").unwrap();
        let mut t = FakeTransport::default();
        transfer(&mut t, &plan, "/srv/app", true).unwrap();

        assert_eq!(*t.cleaned.borrow(), vec!["/srv/app".to_string()]);
        assert_eq!(*t.uploaded.borrow(), vec!["/srv/app/a.txt".to_string()]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib deploy`
Expected: FAIL — `transfer` not defined.

- [ ] **Step 3: Implement `transfer` + `run` pipeline**

Add to `src/commands/deploy.rs`:
```rust
use indicatif::{ProgressBar, ProgressStyle};

/// Run the destructive transfer against any Transport.
pub fn transfer(
    t: &mut dyn Transport,
    plan: &[UploadItem],
    remote_dir: &str,
    clean_remote: bool,
) -> Result<()> {
    validate_remote_dir(remote_dir)?;
    if clean_remote {
        tracing::info!("limpando remoto {}", remote_dir);
        t.clean_remote_dir(remote_dir)?;
    }
    t.ensure_dir(remote_dir)?;

    let bar = ProgressBar::new(plan.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("{bar:40} {pos}/{len} {msg}")
            .unwrap(),
    );
    for item in plan {
        bar.set_message(item.remote.clone());
        t.upload_file(&item.local, &item.remote)?;
        bar.inc(1);
    }
    bar.finish_with_message("upload concluído");
    Ok(())
}

/// Full deploy pipeline. `dry_run` logs the plan and executes nothing destructive.
pub fn run(cfg: &Config, dry_run: bool) -> Result<()> {
    let output_dir = PathBuf::from(&cfg.build.output_dir);

    if dry_run {
        tracing::info!("DRY-RUN — nada será alterado");
        tracing::info!("passo 1/4: rodaria build: {}", cfg.build.command);
        tracing::info!("passo 2/4: limparia remoto: {} (clean_remote={})", cfg.deploy.remote_dir, cfg.deploy.clean_remote);
        if output_dir.exists() {
            let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir)?;
            tracing::info!("passo 3/4: enviaria {} arquivos:", plan.len());
            for item in &plan {
                tracing::info!("  {} -> {}", item.local.display(), item.remote);
            }
        } else {
            tracing::info!("passo 3/4: output_dir '{}' ainda não existe (rode build real para ver arquivos)", output_dir.display());
        }
        tracing::info!("passo 4/4: fim do dry-run");
        return Ok(());
    }

    // 1. delete stale local build
    if output_dir.exists() {
        tracing::info!("passo 1/8: removendo build local antigo '{}'", output_dir.display());
        std::fs::remove_dir_all(&output_dir)
            .with_context(|| format!("falha ao remover {}", output_dir.display()))?;
    } else {
        tracing::info!("passo 1/8: sem build local antigo");
    }

    // 2. run build
    tracing::info!("passo 2/8: rodando build: {}", cfg.build.command);
    run_build(&cfg.build.command)?;

    // 3. verify output
    tracing::info!("passo 3/8: verificando output_dir");
    if !output_dir.exists() {
        bail!("output_dir '{}' não existe após build", output_dir.display());
    }
    let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir)?;
    tracing::info!("passo 4/8: {} arquivos para enviar", plan.len());

    // 4. connect
    tracing::info!("passo 5/8: conectando em {}@{}:{}", cfg.user, cfg.host, cfg.port);
    let mut transport = crate::ssh::Ssh2Transport::connect(cfg)?;

    // 5-7. clean + ensure + upload
    tracing::info!("passo 6/8: limpando remoto (clean_remote={})", cfg.deploy.clean_remote);
    tracing::info!("passo 7/8: enviando arquivos");
    transfer(&mut transport, &plan, &cfg.deploy.remote_dir, cfg.deploy.clean_remote)?;

    tracing::info!("passo 8/8: concluído");
    println!("deploy ok");
    Ok(())
}

fn run_build(command: &str) -> Result<()> {
    let status = if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", command]).status()
    } else {
        std::process::Command::new("sh").args(["-c", command]).status()
    }
    .with_context(|| format!("falha ao iniciar build: {command}"))?;
    if !status.success() {
        bail!("build falhou (exit {:?})", status.code());
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib deploy`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/commands/deploy.rs
git commit -m "feat: deploy pipeline with dry-run and upload plan"
```

---

## Task 6: Init command

**Files:**
- Modify: `src/commands/init.rs`

- [ ] **Step 1: Implement interactive init**

`src/commands/init.rs`:
```rust
use crate::config::{Auth, Build, Config, Deploy, CONFIG_PATH};
use anyhow::{Context, Result};
use dialoguer::{Confirm, Input, Password, Select};
use std::path::Path;

pub fn run() -> Result<()> {
    let path = Path::new(CONFIG_PATH);
    if path.exists()
        && !Confirm::new()
            .with_prompt(format!("{CONFIG_PATH} já existe. Sobrescrever?"))
            .default(false)
            .interact()?
    {
        println!("init cancelado");
        return Ok(());
    }

    let host: String = Input::new().with_prompt("Host / IP da VPS").interact_text()?;
    let port: u16 = Input::new().with_prompt("Porta SSH").default(22).interact_text()?;
    let user: String = Input::new().with_prompt("Usuário").interact_text()?;

    let auth_kind = Select::new()
        .with_prompt("Tipo de autenticação")
        .items(&["chave (.pem)", "senha"])
        .default(0)
        .interact()?;
    let auth = if auth_kind == 0 {
        let key_path: String = Input::new()
            .with_prompt("Caminho da chave")
            .default("~/.ssh/id_rsa".into())
            .interact_text()?;
        Auth::Key { key_path }
    } else {
        let password = Password::new().with_prompt("Senha").interact()?;
        eprintln!("AVISO: a senha será salva em texto puro em {CONFIG_PATH}");
        Auth::Password { password }
    };

    let command: String = Input::new()
        .with_prompt("Comando de build")
        .default("npm run build".into())
        .interact_text()?;
    let output_dir: String = Input::new()
        .with_prompt("Pasta de output do build")
        .default("dist".into())
        .interact_text()?;
    let remote_dir: String = Input::new()
        .with_prompt("Pasta destino na VPS")
        .interact_text()?;
    let clean_remote = Confirm::new()
        .with_prompt("Apagar arquivos remotos antes de enviar?")
        .default(true)
        .interact()?;

    let cfg = Config {
        host,
        port,
        user,
        auth,
        build: Build { command, output_dir },
        deploy: Deploy { remote_dir, clean_remote },
    };
    cfg.save(path)?;
    ensure_gitignore().context("falha ao atualizar .gitignore")?;
    println!("config salva em {CONFIG_PATH}");
    Ok(())
}

/// Make sure `.da/` is git-ignored so secrets are not committed.
fn ensure_gitignore() -> Result<()> {
    let gi = Path::new(".gitignore");
    let current = std::fs::read_to_string(gi).unwrap_or_default();
    if current.lines().any(|l| l.trim() == ".da/") {
        return Ok(());
    }
    let mut new = current.clone();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(".da/\n");
    std::fs::write(gi, new)?;
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: builds.

- [ ] **Step 3: Commit**

```bash
git add src/commands/init.rs
git commit -m "feat: interactive init command"
```

---

## Task 7: CLI wiring + logging + main

**Files:**
- Modify: `src/cli.rs`, `src/main.rs`

- [ ] **Step 1: Define CLI in `src/cli.rs`**

```rust
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
```

- [ ] **Step 2: Implement `src/main.rs`**

```rust
mod cli;
mod commands;
mod config;
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
    init_logging(cli.verbose);

    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Deploy { dry_run } => run_deploy(dry_run),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_deploy(dry_run: bool) -> Result<()> {
    let cfg = Config::load(Path::new(config::CONFIG_PATH))?;
    commands::deploy::run(&cfg, dry_run)
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
```

- [ ] **Step 3: Verify build + all tests**

Run: `cargo build && cargo test`
Expected: builds; all unit tests pass.

- [ ] **Step 4: Smoke test CLI**

Run: `cargo run -- --help` then `cargo run -- deploy --dry-run` (expect config-missing error suggesting `da init`).
Expected: help prints; dry-run without config exits 1 with clear stderr message.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: wire cli, logging, and main dispatch"
```

---

## Self-Review Notes

- **Spec coverage:** init (Task 6), deploy pipeline 8 steps (Task 5), dry-run (Task 5), logging to stderr + redacted password (Task 5/7 — password never passed to a log macro), host+port (Task 2), key/password auth (Task 2/4), `~` expansion (Task 2), `.gitignore` (Task 6), remote_dir safety (Task 3), self-contained binary via vendored-openssl (Task 1). All covered.
- **Type consistency:** `Transport` methods (`clean_remote_dir`, `ensure_dir`, `upload_file`) identical in trait, ssh2 impl, fake, and `transfer`. `Config`/`Auth`/`Build`/`Deploy` field names consistent across tasks.
- **Note:** `ChronoLocal` requires `tracing-subscriber` feature `chrono` — if compile fails, swap to `.with_timer(tracing_subscriber::fmt::time::uptime())` or remove the timer. Adjust during execution.
