use crate::config::Config;
use crate::ssh::{Transport, validate_remote_dir};
use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
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

/// Run the destructive transfer against any Transport.
pub fn transfer(
    t: &mut dyn Transport,
    plan: &[UploadItem],
    remote_dir: &str,
    clean_remote: bool,
) -> Result<()> {
    validate_remote_dir(remote_dir)?;
    if clean_remote {
        t.clean_remote_dir(remote_dir)?;
    }
    t.ensure_dir(remote_dir)?;

    let bar = ProgressBar::new(plan.len() as u64);
    bar.set_style(
        ProgressStyle::with_template("  {bar:40.cyan/blue} {pos}/{len} {wide_msg}").unwrap(),
    );
    for item in plan {
        bar.set_message(item.remote.clone());
        t.upload_file(&item.local, &item.remote)?;
        bar.inc(1);
    }
    bar.finish_and_clear();
    Ok(())
}

/// Full deploy pipeline. `dry_run` logs the plan and executes nothing destructive.
/// `verbose` streams the build output live instead of capturing it.
pub fn run(cfg: &Config, dry_run: bool, verbose: bool) -> Result<()> {
    let output_dir = PathBuf::from(&cfg.build.output_dir);
    let target = format!("{}@{}:{}", cfg.user, cfg.host, cfg.port);

    if dry_run {
        tracing::info!("dry-run — nada será alterado");
        tracing::info!("  build:        {}", cfg.build.command);
        tracing::info!("  destino:      {}{}", target, cfg.deploy.remote_dir);
        tracing::info!("  limpar antes: {}", cfg.deploy.clean_remote);
        if output_dir.exists() {
            let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir)?;
            tracing::info!(
                "  enviaria {} arquivo(s) de '{}':",
                plan.len(),
                output_dir.display()
            );
            for item in &plan {
                tracing::info!("    -> {}", item.remote);
            }
        } else {
            tracing::info!(
                "  '{}' ainda não existe — rode um build real para listar os arquivos",
                output_dir.display()
            );
        }
        return Ok(());
    }

    // [1/5] clear stale local build
    if output_dir.exists() {
        tracing::info!("[1/5] limpando build local antigo: {}", output_dir.display());
        std::fs::remove_dir_all(&output_dir)
            .with_context(|| format!("falha ao remover {}", output_dir.display()))?;
    } else {
        tracing::info!("[1/5] nenhum build local antigo");
    }

    // [2/5] build + verify output
    tracing::info!("[2/5] build: {}", cfg.build.command);
    let build_start = std::time::Instant::now();
    run_build(&cfg.build.command, verbose)?;
    if !output_dir.exists() {
        bail!(
            "build terminou mas a pasta '{}' não foi gerada",
            output_dir.display()
        );
    }
    let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir)?;
    tracing::info!(
        "      build concluído em {:.1}s — {} arquivo(s)",
        build_start.elapsed().as_secs_f64(),
        plan.len()
    );

    // [3/5] connect
    tracing::info!("[3/5] conectando em {target}");
    let mut transport = crate::ssh::Ssh2Transport::connect(cfg)?;

    // [4/5] clean remote
    if cfg.deploy.clean_remote {
        tracing::info!("[4/5] limpando pasta remota: {}", cfg.deploy.remote_dir);
    } else {
        tracing::info!("[4/5] mantendo arquivos remotos (clean_remote=false)");
    }

    // [5/5] upload
    tracing::info!(
        "[5/5] enviando {} arquivo(s) para {}",
        plan.len(),
        cfg.deploy.remote_dir
    );
    let upload_start = std::time::Instant::now();
    transfer(
        &mut transport,
        &plan,
        &cfg.deploy.remote_dir,
        cfg.deploy.clean_remote,
    )?;

    tracing::info!(
        "✓ deploy concluído — {} arquivo(s) em {:.1}s para {}{}",
        plan.len(),
        upload_start.elapsed().as_secs_f64(),
        target,
        cfg.deploy.remote_dir
    );
    Ok(())
}

/// Run the local build command. In verbose mode the build output streams to
/// the terminal; otherwise it is captured and only shown if the build fails.
fn run_build(command: &str, verbose: bool) -> Result<()> {
    use std::process::{Command, Stdio};

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if verbose {
        let status = cmd
            .status()
            .with_context(|| format!("falha ao iniciar build: {command}"))?;
        if !status.success() {
            bail!("build falhou (código {:?})", status.code());
        }
        return Ok(());
    }

    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("falha ao iniciar build: {command}"))?;
    if !out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "build falhou (código {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            out.status.code(),
            stdout.trim(),
            stderr.trim()
        );
    }
    Ok(())
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
        assert_eq!(
            remotes,
            vec!["/var/www/app/index.html", "/var/www/app/sub/app.js"]
        );
    }

    #[test]
    fn plan_errors_when_missing() {
        let missing = std::env::temp_dir().join("da_test_missing_xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(build_upload_plan(&missing, "/x").is_err());
    }

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
