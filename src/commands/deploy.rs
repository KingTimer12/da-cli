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
        tracing::info!("limpando remoto {}", remote_dir);
        t.clean_remote_dir(remote_dir)?;
    }
    t.ensure_dir(remote_dir)?;

    let bar = ProgressBar::new(plan.len() as u64);
    bar.set_style(ProgressStyle::with_template("{bar:40} {pos}/{len} {msg}").unwrap());
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
        tracing::info!(
            "passo 2/4: limparia remoto: {} (clean_remote={})",
            cfg.deploy.remote_dir,
            cfg.deploy.clean_remote
        );
        if output_dir.exists() {
            let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir)?;
            tracing::info!("passo 3/4: enviaria {} arquivos:", plan.len());
            for item in &plan {
                tracing::info!("  {} -> {}", item.local.display(), item.remote);
            }
        } else {
            tracing::info!(
                "passo 3/4: output_dir '{}' ainda não existe (rode build real para ver arquivos)",
                output_dir.display()
            );
        }
        tracing::info!("passo 4/4: fim do dry-run");
        return Ok(());
    }

    // 1. delete stale local build
    if output_dir.exists() {
        tracing::info!(
            "passo 1/8: removendo build local antigo '{}'",
            output_dir.display()
        );
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
    tracing::info!(
        "passo 5/8: conectando em {}@{}:{}",
        cfg.user,
        cfg.host,
        cfg.port
    );
    let mut transport = crate::ssh::Ssh2Transport::connect(cfg)?;

    // 5-7. clean + ensure + upload
    tracing::info!(
        "passo 6/8: limpando remoto (clean_remote={})",
        cfg.deploy.clean_remote
    );
    tracing::info!("passo 7/8: enviando arquivos");
    transfer(
        &mut transport,
        &plan,
        &cfg.deploy.remote_dir,
        cfg.deploy.clean_remote,
    )?;

    tracing::info!("passo 8/8: concluído");
    println!("deploy ok");
    Ok(())
}

fn run_build(command: &str) -> Result<()> {
    let status = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", command])
            .status()
    } else {
        std::process::Command::new("sh")
            .args(["-c", command])
            .status()
    }
    .with_context(|| format!("falha ao iniciar build: {command}"))?;
    if !status.success() {
        bail!("build falhou (exit {:?})", status.code());
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
