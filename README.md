# DA — Deploy Automático

CLI em Rust para automatizar o deploy de frontend para uma VPS via SSH/SFTP.
O objetivo é reduzir todo o processo manual (rebuild + upload) a um único
comando: `da deploy`.

## Motivação

O projeto não tenta substituir um pipeline de CI/CD — é uma forma rápida de
fazer deploy direto, sem CI/CD, para contextos onde montar/usar um pipeline
não é desejado. A dor concreta que ele resolve: atualizar o frontend
manualmente (build + subir arquivo por arquivo) interrompe o raciocínio
lógico no meio de uma tarefa. `da deploy` colapsa isso num comando só, para
o foco ficar no código e não no processo de publicação.

## O que faz

Ao rodar `da deploy`, o seguinte pipeline é executado:

1. Apaga a pasta de build local antiga
2. Roda o comando de build
3. Conecta na VPS (verificando o host key)
4. Apaga os arquivos dentro da pasta de destino na VPS
5. Transfere o build novo para a VPS

A transferência empacota a pasta de build num `tar.gz` em memória e o
envia/extrai num único round-trip SSH (`tar -xzf - -C destino`), em vez de
enviar arquivo por arquivo. Isso é muito mais rápido para builds com muitos
arquivos pequenos. **Requisito:** o servidor precisa ter o comando `tar`
(presente por padrão em qualquer Linux).

## Instalação

Requer Rust (edition 2024). O binário é autossuficiente — `libssh2` e
`openssl` são compilados junto (feature `vendored-openssl`), sem
dependências de sistema.

```bash
cargo build --release
# binário gerado em:
./target/release/da
```

Opcional, instalar no PATH:

```bash
cargo install --path .
```

## Uso

### `da init`

Cria o arquivo de configuração `.da/config.toml` de forma interativa.
Pergunta host, porta, usuário, tipo de autenticação (chave `.pem` ou
senha), comando de build, pasta de output, pasta de destino na VPS e se
deve limpar o remoto antes de enviar.

```bash
da init
```

Ao terminar, adiciona `.da/` ao `.gitignore` automaticamente (para não
vazar credenciais) e grava o config com permissão `0600`.

### `da deploy`

Roda o pipeline completo.

```bash
da deploy
```

### `da deploy --dry-run`

Mostra o que *seria* feito sem executar nada destrutivo: não roda build,
não apaga nada, não envia. Útil para conferir a config antes de um deploy
real.

```bash
da deploy --dry-run
```

### `-v` / `--verbose`

Aumenta o log para nível debug (detalhes de SSH, listagem de arquivos).
Disponível em qualquer comando.

```bash
da deploy -v
```

## Configuração — `.da/config.toml`

```toml
host = "1.2.3.4"
port = 22
user = "ubuntu"

# Autenticação: exatamente um dos dois tipos.

# Opção A — chave (.pem):
[auth]
type = "key"
key_path = "~/.ssh/me.pem"   # ~ é expandido para o home

# Opção B — senha:
# [auth]
# type = "password"
# password = "..."           # salva em texto puro (ver Segurança)

[build]
command = "npm run build"    # roda no shell local, na raiz do projeto
output_dir = "dist"          # pasta gerada pelo build (relativa à raiz)

[deploy]
remote_dir = "/var/www/app"  # pasta de destino na VPS
clean_remote = true          # apaga conteúdo do remoto antes (default: true)
```

## Logging

Todos os passos são logados em **stderr**, com timestamp e nível
(stdout fica limpo para scripting). Formato:

```
[12:01:03  INFO] passo 3/8: rodando build: npm run build
```

A senha **nunca** é logada. A autenticação loga apenas o tipo (`key` /
`password`) e, no caso de chave, o caminho.

## Segurança

- **Permissões do config:** `.da/config.toml` é gravado com `0600` e a
  pasta `.da/` com `0700` (Unix), pois o arquivo pode conter senha em
  texto puro. Use autenticação por chave `.pem` quando possível.
- **Verificação de host key (anti-MITM):** no primeiro acesso a uma
  máquina, o fingerprint SHA256 é exibido e pedida confirmação antes de
  salvar em `~/.ssh/known_hosts`. Em acessos seguintes, a chave é
  validada; se mudar (possível MITM), o deploy é abortado.
- **Proteção do remoto:** `clean_remote` apaga apenas o *conteúdo* de
  `remote_dir`, nunca a própria pasta, e recusa `/` ou caminho vazio.

## Desenvolvimento

```bash
cargo test          # testes unitários
cargo build         # build de debug
cargo build --release
```

O perfil de release é otimizado para tamanho (LTO, `codegen-units = 1`,
`strip`, `panic = abort`).
