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
4. Salva um backup do conteúdo remoto atual em `.da/backups/`
5. Apaga os arquivos dentro da pasta de destino na VPS
6. Transfere o build novo para a VPS

A transferência empacota a pasta de build num `tar.gz` em memória e o
envia/extrai num único round-trip SSH (`tar -xzf - -C destino`), em vez de
enviar arquivo por arquivo. Isso é muito mais rápido para builds com muitos
arquivos pequenos. **Requisito:** o servidor precisa ter o comando `tar`
(presente por padrão em qualquer Linux).

Recursos relacionados, detalhados abaixo:

- **`da undo`** — reverte o último deploy a partir do backup salvo no passo 4.
- **`.daignore`** — controla o que não é enviado (client) e o que não é
  apagado no remoto (server), com sintaxe estilo `.gitignore`.
- **`da upgrade`** — atualiza o próprio binário a partir da última release.

## Instalação

Binários prontos para Linux, macOS (Intel e Apple Silicon) e Windows são
publicados em [Releases](https://github.com/KingTimer12/da-cli/releases). O
binário é autossuficiente — `libssh2` e `openssl` são compilados junto, sem
dependências de sistema.

### Linux / macOS (curl ou wget)

```bash
curl -fsSL https://raw.githubusercontent.com/KingTimer12/da-cli/master/install.sh | sh
```

Instala em `~/.local/bin` por padrão. Variáveis opcionais: `DA_VERSION`
(ex: `v0.1.0`) e `DA_INSTALL_DIR`.

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/KingTimer12/da-cli/master/install.ps1 | iex
```

Instala em `%LOCALAPPDATA%\Programs\da` e adiciona ao PATH do usuário
(reabra o terminal depois).

### A partir do código

Requer Rust (edition 2024).

```bash
cargo install --path .
# ou:
cargo build --release   # binário em ./target/release/da
```

> Os one-liners apontam para o branch `master`. Se o branch padrão do repo for
> outro (ex: `master`), ajuste a URL.

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

### `da undo`

Antes de limpar o remoto, todo `da deploy` salva um backup (`.tar.gz`) do que
estava lá em `.da/backups/`. Se o deploy mandou pra pasta errada ou apagou algo
indevido, `da undo` restaura o backup mais recente:

```bash
da undo                       # restaura o backup mais recente
da undo --id 20260625-143000  # restaura um backup específico
da undo --dry-run             # mostra o que seria restaurado
```

Mantém os 5 backups mais recentes (os antigos são apagados). Liste os ids com
`ls .da/backups/`.

### `da upgrade`

Baixa a última release do GitHub e troca o binário em uso, no lugar:

```bash
da upgrade           # atualiza se houver versão nova
da upgrade --force   # reinstala mesmo já estando na última
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

## Ignorar arquivos — `.daignore`

Três arquivos opcionais na raiz do projeto controlam o que é enviado e o que é
apagado, com sintaxe **estilo `.gitignore`** (globs, `pasta/`, `/ancorado`):

| Arquivo            | Efeito                                                              |
|--------------------|--------------------------------------------------------------------|
| `.daignore`        | Geral: não envia (client) **e** não apaga no remoto (server).      |
| `.daignore.client` | Só client: não inclui esses arquivos no pacote enviado.            |
| `.daignore.server` | Só server: não apaga esses arquivos no remoto ao limpar.          |

Exemplo — não enviar source maps, e nunca apagar a pasta de uploads do servidor:

```gitignore
# .daignore.client
*.map

# .daignore.server
uploads/
```

Quando há regras de server, a limpeza deixa de ser um wipe completo: o DA lista
os arquivos remotos e apaga só os não-protegidos.

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

## Publicação (release)

O CI (`.github/workflows/release.yml`) builda os binários das 4 plataformas e
anexa os artefatos (`.tar.gz` / `.zip` + `.sha256`) a um GitHub Release quando
uma tag `v*` é enviada:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Os scripts `install.sh` / `install.ps1` baixam o último release
automaticamente. (Pacote `winget` não está incluído — exige submeter um
manifesto ao `microsoft/winget-pkgs`; o instalador `irm` cobre o Windows por
enquanto.)

## Desenvolvimento

```bash
cargo test          # testes unitários
cargo build         # build de debug
cargo build --release
```

O perfil de release é otimizado para tamanho (LTO, `codegen-units = 1`,
`strip`, `panic = abort`).
