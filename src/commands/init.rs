use crate::config::{Auth, Build, CONFIG_PATH, Config, Deploy};
use anyhow::{Context, Result};
use std::path::Path;

pub fn run() -> Result<()> {
    let path = Path::new(CONFIG_PATH);

    cliclack::intro("da init — configurar deploy")?;

    if path.exists() {
        let overwrite = cliclack::confirm(format!("{CONFIG_PATH} já existe. Sobrescrever?"))
            .initial_value(false)
            .interact()?;
        if !overwrite {
            cliclack::outro("init cancelado")?;
            return Ok(());
        }
    }

    let host: String = cliclack::input("Host / IP da VPS")
        .placeholder("1.2.3.4")
        .interact()?;
    let port: u16 = cliclack::input("Porta SSH")
        .default_input("22")
        .interact()?;
    let user: String = cliclack::input("Usuário")
        .placeholder("root")
        .interact()?;

    let auth_kind: &str = cliclack::select("Tipo de autenticação")
        .item("key", "Chave (.pem / .key)", "recomendado")
        .item("password", "Senha", "salva em texto puro")
        .interact()?;
    let auth = if auth_kind == "key" {
        let key_path: String = cliclack::input("Caminho da chave")
            .default_input("~/.ssh/id_rsa")
            .interact()?;
        Auth::Key { key_path }
    } else {
        let password: String = cliclack::password("Senha").mask('▪').interact()?;
        cliclack::log::warning(format!(
            "a senha será salva em texto puro em {CONFIG_PATH}"
        ))?;
        Auth::Password { password }
    };

    let command: String = cliclack::input("Comando de build")
        .default_input("npm run build")
        .interact()?;
    let output_dir: String = cliclack::input("Pasta de output do build")
        .default_input("dist")
        .interact()?;
    let remote_dir: String = cliclack::input("Pasta destino na VPS")
        .placeholder("/var/www/app")
        .interact()?;
    let clean_remote = cliclack::confirm("Apagar arquivos remotos antes de enviar?")
        .initial_value(true)
        .interact()?;

    let cfg = Config {
        host,
        port,
        user,
        auth,
        build: Build {
            command,
            output_dir,
        },
        deploy: Deploy {
            remote_dir,
            clean_remote,
        },
    };
    cfg.save(path)?;
    ensure_gitignore().context("falha ao atualizar .gitignore")?;

    cliclack::outro(format!("config salva em {CONFIG_PATH} — rode `da deploy`"))?;
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
