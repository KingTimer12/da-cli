//! `da upgrade` — baixa a última release do GitHub e troca o binário local.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;

const REPO: &str = "KingTimer12/da-cli";
const UA: &str = concat!("da-cli/", env!("CARGO_PKG_VERSION"));

/// Alvo de build da plataforma atual + extensão do arquivo de release.
fn target() -> Result<(&'static str, &'static str)> {
    let t = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        ("x86_64-unknown-linux-gnu", "tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        ("aarch64-apple-darwin", "tar.gz")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        ("x86_64-apple-darwin", "tar.gz")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        ("x86_64-pc-windows-msvc", "zip")
    } else {
        bail!("plataforma sem release pré-compilada — instale via `cargo install`");
    };
    Ok(t)
}

fn bin_name() -> &'static str {
    if cfg!(windows) { "da.exe" } else { "da" }
}

fn http_bytes(url: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let resp = ureq::get(url)
        .set("User-Agent", UA)
        .call()
        .with_context(|| format!("falha ao baixar {url}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .context("falha ao ler resposta")?;
    Ok(buf)
}

/// Versão da última release (tag sem o 'v' inicial).
fn latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let json: serde_json::Value = ureq::get(&url)
        .set("User-Agent", UA)
        .call()
        .context("falha ao consultar GitHub")?
        .into_json()
        .context("resposta do GitHub inválida")?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("release sem tag_name"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Extrai o binário do arquivo baixado e devolve um caminho temporário pronto.
fn extract_binary(archive: &[u8], tmp_dir: &std::path::Path) -> Result<PathBuf> {
    let out = tmp_dir.join(bin_name());
    extract_into(archive, &out)?;
    if !out.exists() {
        bail!("binário '{}' não encontrado no arquivo de release", bin_name());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(out)
}

#[cfg(not(windows))]
fn extract_into(archive: &[u8], out: &std::path::Path) -> Result<()> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(gz);
    for entry in tar.entries().context("tar inválido")? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.file_name().and_then(|n| n.to_str()) == Some(bin_name()) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            std::fs::write(out, bytes)?;
            return Ok(());
        }
    }
    bail!("binário não encontrado no tar")
}

#[cfg(windows)]
fn extract_into(archive: &[u8], out: &std::path::Path) -> Result<()> {
    use std::io::Read;
    let reader = std::io::Cursor::new(archive);
    let mut zip = zip::ZipArchive::new(reader).context("zip inválido")?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let name = file.name().to_string();
        if name.ends_with(bin_name()) {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            std::fs::write(out, bytes)?;
            return Ok(());
        }
    }
    bail!("binário não encontrado no zip")
}

pub fn run(force: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    tracing::info!("[1/4] versão atual: {current} — consultando GitHub");

    let latest = latest_version()?;
    if latest == current && !force {
        tracing::info!("✓ já está na última versão ({current})");
        return Ok(());
    }
    if latest == current {
        tracing::info!("[2/4] reinstalando versão {latest} (--force)");
    } else {
        tracing::info!("[2/4] nova versão disponível: {latest}");
    }

    let (tgt, ext) = target()?;
    let url = format!(
        "https://github.com/{REPO}/releases/download/v{latest}/da-{tgt}.{ext}"
    );
    tracing::info!("[3/4] baixando {url}");
    let archive = http_bytes(&url)?;

    let tmp = std::env::temp_dir().join(format!("da-upgrade-{latest}"));
    std::fs::create_dir_all(&tmp).context("falha ao criar dir temporário")?;
    let new_bin = extract_binary(&archive, &tmp)?;

    tracing::info!("[4/4] substituindo binário em uso");
    self_replace::self_replace(&new_bin).context("falha ao substituir o binário")?;
    let _ = std::fs::remove_dir_all(&tmp);

    tracing::info!("✓ atualizado para {latest}");
    Ok(())
}
