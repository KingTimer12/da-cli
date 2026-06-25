use crate::config::Config;
use crate::ignore::DaIgnore;
use crate::ssh::Transport;
use anyhow::{Context, Result, bail};
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
/// Arquivos cobertos pelo `.daignore`/`.daignore.client` são omitidos.
pub fn build_upload_plan(
    output_dir: &Path,
    remote_dir: &str,
    ignore: &DaIgnore,
) -> Result<Vec<UploadItem>> {
    if !output_dir.exists() {
        bail!("output_dir não existe: {}", output_dir.display());
    }
    let mut items = Vec::new();
    for entry in WalkDir::new(output_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(output_dir).unwrap();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if ignore.client_excludes(&rel_str) {
                continue;
            }
            items.push(UploadItem {
                local: entry.path().to_path_buf(),
                remote: format!("{}/{}", remote_dir.trim_end_matches('/'), rel_str),
            });
        }
    }
    items.sort_by(|a, b| a.remote.cmp(&b.remote));
    Ok(items)
}

/// Pack `output_dir` into an in-memory gzip'd tar. File paths are stored
/// relative to `output_dir` so the archive extracts straight into the remote
/// dir. Compression level 1 favors speed (assets are already small/compressed).
pub fn make_tar_gz(output_dir: &Path, ignore: &DaIgnore) -> Result<Vec<u8>> {
    use flate2::{Compression, write::GzEncoder};
    if !output_dir.exists() {
        bail!("output_dir não existe: {}", output_dir.display());
    }
    let encoder = GzEncoder::new(Vec::new(), Compression::new(1));
    let mut builder = tar::Builder::new(encoder);
    // Adiciona arquivo por arquivo pra poder pular os ignorados pelo client.
    for entry in WalkDir::new(output_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(output_dir).unwrap();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if ignore.client_excludes(&rel_str) {
            continue;
        }
        let mut f = std::fs::File::open(entry.path())
            .with_context(|| format!("falha ao abrir {}", entry.path().display()))?;
        builder
            .append_file(&rel_str, &mut f)
            .with_context(|| format!("falha ao empacotar {rel_str}"))?;
    }
    let encoder = builder.into_inner().context("falha ao fechar tar")?;
    let bytes = encoder.finish().context("falha ao comprimir tar")?;
    Ok(bytes)
}

/// Limpa o remoto respeitando `.daignore`/`.daignore.server`: se houver regras
/// de proteção, apaga só os arquivos não-protegidos; senão faz wipe completo.
pub fn clean_remote(t: &mut dyn Transport, remote_dir: &str, ignore: &DaIgnore) -> Result<usize> {
    if !ignore.has_server_rules() {
        t.clean_remote_dir(remote_dir)?;
        return Ok(0);
    }
    let files = t.list_remote_files(remote_dir)?;
    let mut protected = 0usize;
    let to_delete: Vec<String> = files
        .into_iter()
        .filter(|rel| {
            if ignore.server_protects(rel) {
                protected += 1;
                false
            } else {
                true
            }
        })
        .collect();
    t.delete_remote_paths(remote_dir, &to_delete)?;
    Ok(protected)
}

/// Full deploy pipeline. `dry_run` logs the plan and executes nothing destructive.
/// `verbose` streams the build output live instead of capturing it.
pub fn run(cfg: &Config, dry_run: bool, verbose: bool) -> Result<()> {
    let output_dir = PathBuf::from(&cfg.build.output_dir);
    let target = format!("{}@{}:{}", cfg.user, cfg.host, cfg.port);
    let ignore = DaIgnore::load(Path::new("."))?;
    if !ignore.loaded.is_empty() {
        tracing::info!("regras .daignore ativas: {}", ignore.loaded.join(", "));
    }

    if dry_run {
        tracing::info!("dry-run — nada será alterado");
        tracing::info!("  build:        {}", cfg.build.command);
        tracing::info!("  destino:      {}{}", target, cfg.deploy.remote_dir);
        tracing::info!("  limpar antes: {}", cfg.deploy.clean_remote);
        if ignore.has_server_rules() {
            tracing::info!("  remoto: arquivos protegidos por .daignore.server não serão apagados");
        }
        if output_dir.exists() {
            let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir, &ignore)?;
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

    // [1/6] clear stale local build
    if output_dir.exists() {
        tracing::info!("[1/6] limpando build local antigo: {}", output_dir.display());
        std::fs::remove_dir_all(&output_dir)
            .with_context(|| format!("falha ao remover {}", output_dir.display()))?;
    } else {
        tracing::info!("[1/6] nenhum build local antigo");
    }

    // [2/6] build + verify output
    tracing::info!("[2/6] build: {}", cfg.build.command);
    let build_start = std::time::Instant::now();
    run_build(&cfg.build.command, verbose)?;
    if !output_dir.exists() {
        bail!(
            "build terminou mas a pasta '{}' não foi gerada",
            output_dir.display()
        );
    }
    let plan = build_upload_plan(&output_dir, &cfg.deploy.remote_dir, &ignore)?;
    let archive = make_tar_gz(&output_dir, &ignore)?;
    tracing::info!(
        "      build concluído em {:.1}s — {} arquivo(s), pacote {}",
        build_start.elapsed().as_secs_f64(),
        plan.len(),
        human_size(archive.len())
    );

    // [3/6] connect
    tracing::info!("[3/6] conectando em {target}");
    let mut transport = crate::ssh::Ssh2Transport::connect(cfg)?;

    // [4/6] backup do conteúdo remoto antes de qualquer remoção
    if cfg.deploy.clean_remote {
        tracing::info!("[4/6] salvando backup do remoto antes de limpar");
        let snapshot = transport.download_archive(&cfg.deploy.remote_dir)?;
        let id = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        match crate::commands::backup::save(&snapshot, &id)? {
            Some(p) => tracing::info!(
                "      backup salvo: {} ({})",
                p.display(),
                human_size(snapshot.len())
            ),
            None => tracing::info!("      remoto vazio — nada pra fazer backup"),
        }
    } else {
        tracing::info!("[4/6] clean_remote=false — sem backup (nada será apagado)");
    }

    // [5/6] clean remote (respeitando .daignore.server)
    if cfg.deploy.clean_remote {
        tracing::info!("[5/6] limpando pasta remota: {}", cfg.deploy.remote_dir);
        let protected = clean_remote(&mut transport, &cfg.deploy.remote_dir, &ignore)?;
        if protected > 0 {
            tracing::info!("      {protected} arquivo(s) preservados por .daignore.server");
        }
    } else {
        tracing::info!("[5/6] mantendo arquivos remotos (clean_remote=false)");
    }

    // [6/6] upload + extract (single round-trip)
    tracing::info!(
        "[6/6] enviando pacote ({}) para {}",
        human_size(archive.len()),
        cfg.deploy.remote_dir
    );
    let upload_start = std::time::Instant::now();
    transport.upload_archive(&archive, &cfg.deploy.remote_dir)?;

    tracing::info!(
        "✓ deploy concluído — {} arquivo(s) em {:.1}s para {}{}",
        plan.len(),
        upload_start.elapsed().as_secs_f64(),
        target,
        cfg.deploy.remote_dir
    );
    Ok(())
}

pub fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
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

    /// `.daignore` vazio (não ignora nada) a partir de um dir temporário.
    fn empty_ignore() -> DaIgnore {
        let dir = std::env::temp_dir().join("da_test_noignore");
        let _ = std::fs::create_dir_all(&dir);
        DaIgnore::load(&dir).unwrap()
    }

    #[test]
    fn plan_maps_nested_files_to_remote() {
        let tmp = std::env::temp_dir().join("da_test_plan");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("index.html"), "x").unwrap();
        std::fs::write(tmp.join("sub/app.js"), "y").unwrap();

        let plan = build_upload_plan(&tmp, "/var/www/app", &empty_ignore()).unwrap();
        let remotes: Vec<_> = plan.iter().map(|i| i.remote.as_str()).collect();
        assert_eq!(
            remotes,
            vec!["/var/www/app/index.html", "/var/www/app/sub/app.js"]
        );
    }

    #[test]
    fn plan_excludes_client_ignored() {
        // Raiz do projeto (com .daignore.client) separada da pasta de build.
        let root = std::env::temp_dir().join("da_test_plan_ignore");
        let _ = std::fs::remove_dir_all(&root);
        let out = root.join("dist");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("app.js"), "x").unwrap();
        std::fs::write(out.join("app.js.map"), "y").unwrap();
        std::fs::write(root.join(crate::ignore::CLIENT), "*.map\n").unwrap();
        let ig = DaIgnore::load(&root).unwrap();

        let plan = build_upload_plan(&out, "/srv", &ig).unwrap();
        let remotes: Vec<_> = plan.iter().map(|i| i.remote.as_str()).collect();
        assert_eq!(remotes, vec!["/srv/app.js"]);
    }

    #[test]
    fn plan_errors_when_missing() {
        let missing = std::env::temp_dir().join("da_test_missing_xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(build_upload_plan(&missing, "/x", &empty_ignore()).is_err());
    }

    #[derive(Default)]
    struct FakeTransport {
        cleaned: RefCell<Vec<String>>,
        archives: RefCell<Vec<(usize, String)>>,
        deleted: RefCell<Vec<String>>,
        remote_files: Vec<String>,
    }
    impl Transport for FakeTransport {
        fn clean_remote_dir(&mut self, d: &str) -> Result<()> {
            self.cleaned.borrow_mut().push(d.to_string());
            Ok(())
        }
        fn upload_archive(&mut self, archive: &[u8], remote_dir: &str) -> Result<()> {
            self.archives
                .borrow_mut()
                .push((archive.len(), remote_dir.to_string()));
            Ok(())
        }
        fn download_archive(&mut self, _remote_dir: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn list_remote_files(&mut self, _remote_dir: &str) -> Result<Vec<String>> {
            Ok(self.remote_files.clone())
        }
        fn delete_remote_paths(&mut self, _remote_dir: &str, paths: &[String]) -> Result<()> {
            self.deleted.borrow_mut().extend(paths.iter().cloned());
            Ok(())
        }
    }

    #[test]
    fn upload_archive_sends_to_remote() {
        let mut t = FakeTransport::default();
        t.upload_archive(&[1u8, 2, 3, 4], "/srv/app").unwrap();
        assert_eq!(*t.archives.borrow(), vec![(4, "/srv/app".to_string())]);
    }

    #[test]
    fn clean_remote_full_wipe_without_server_rules() {
        let mut t = FakeTransport::default();
        let n = clean_remote(&mut t, "/srv", &empty_ignore()).unwrap();
        assert_eq!(n, 0);
        assert_eq!(*t.cleaned.borrow(), vec!["/srv".to_string()]);
        assert!(t.deleted.borrow().is_empty());
    }

    #[test]
    fn clean_remote_preserves_server_protected() {
        let dir = std::env::temp_dir().join("da_test_srvprotect");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::ignore::SERVER), "uploads/\n").unwrap();
        let ig = DaIgnore::load(&dir).unwrap();

        let mut t = FakeTransport::default();
        t.remote_files = vec![
            "index.html".into(),
            "uploads/photo.png".into(),
            "old.js".into(),
        ];
        let protected = clean_remote(&mut t, "/srv", &ig).unwrap();
        assert_eq!(protected, 1);
        assert!(t.cleaned.borrow().is_empty(), "não deve usar wipe completo");
        let mut deleted = t.deleted.borrow().clone();
        deleted.sort();
        assert_eq!(deleted, vec!["index.html".to_string(), "old.js".to_string()]);
    }

    #[test]
    fn tar_gz_roundtrips_files() {
        use std::io::Read;
        let tmp = std::env::temp_dir().join("da_test_tar");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("index.html"), "hello").unwrap();
        std::fs::write(tmp.join("sub/app.js"), "code").unwrap();

        let bytes = make_tar_gz(&tmp, &empty_ignore()).unwrap();
        assert!(!bytes.is_empty());

        // Unpack and confirm both files survive with correct relative paths.
        let gz = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(gz);
        let mut names: Vec<String> = archive
            .entries()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.header().entry_type().is_file())
            .map(|mut e| {
                let p = e.path().unwrap().to_string_lossy().replace("./", "");
                let mut s = String::new();
                e.read_to_string(&mut s).ok();
                p
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["index.html".to_string(), "sub/app.js".to_string()]);
    }
}
