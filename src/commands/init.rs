use crate::config::{Auth, Build, CONFIG_PATH, Config, Deploy};
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
    let port: u16 = Input::new()
        .with_prompt("Porta SSH")
        .default(22)
        .interact_text()?;
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
