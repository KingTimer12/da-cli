# DA — Deploy Automático: Design

**Date:** 2026-06-15
**Status:** Approved

## Purpose

CLI in Rust to automate frontend deploy to a production VPS. Today the
slow step is manually rebuilding and uploading the frontend. DA reduces
this to a single command, `da deploy`, which runs the full pipeline:

1. Delete previous local build folder
2. Run the build command
3. Delete remote files inside the target dir on the VPS
4. Transfer the build output to the VPS

Two commands: `da init` (create config) and `da deploy` (run the pipeline).

## Transport Decision

Use the `ssh2` crate (libssh2 binding) with the `vendored-openssl`
feature so the binary is self-contained (no system libssh2/openssl
required). Synchronous API → linear pipeline flow. Supports both password
auth and `.pem` key auth (`userauth_pubkey_file`). SFTP is used for
upload and remote delete.

Rejected: `russh` (pure Rust but async, more boilerplate, overkill);
shelling out to `scp`/`rsync` (depends on system binaries, breaks on
Windows, fragile error parsing).

## Crates

| Purpose            | Crate                          |
|--------------------|--------------------------------|
| Arg parsing        | `clap` (derive, subcommands)   |
| Config (de)serialize | `serde`, `toml`              |
| SSH/SFTP transport | `ssh2` (feature `vendored-openssl`) |
| Errors             | `anyhow`                       |
| Interactive prompts| `dialoguer`                    |
| Progress bar       | `indicatif`                    |
| Logging            | `tracing`, `tracing-subscriber`|
| Recurse local dir  | `walkdir`                      |
| Home expansion     | `dirs` (expand `~` in key_path)|

## Module Layout

```
src/
  main.rs          # init tracing subscriber, clap parse, dispatch
  cli.rs           # Cli + Commands (clap derive); global -v/--verbose
  config.rs        # Config struct, Auth enum, load/save TOML, path expansion
  commands/
    mod.rs
    init.rs        # interactive prompts -> write .da/config.toml
    deploy.rs      # orchestrate pipeline (supports --dry-run)
  ssh.rs           # ssh2 wrapper: connect(auth), clean_remote_dir, upload_dir, mkdir_p
  error.rs         # shared error helpers / exit codes
```

Each unit has one purpose: `config` owns persistence + validation, `ssh`
owns transport, `commands/*` own orchestration, `cli` owns parsing.

## Config — `.da/config.toml`

```toml
host = "1.2.3.4"
port = 22
user = "ubuntu"

[auth]
type = "key"                  # "key" | "password"
key_path = "~/.ssh/me.pem"    # present when type = "key"
# password = "..."            # present when type = "password" (plaintext + warn)

[build]
command = "npm run build"     # runs in local shell, cwd = project root
output_dir = "dist"           # build output folder, relative to project root

[deploy]
remote_dir = "/var/www/app"   # destination on VPS
clean_remote = true           # delete remote contents before upload (default true)
```

### Auth model (Rust)

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Auth {
    Key { key_path: String },
    Password { password: String },
}
```

Exactly one variant. `key_path` may contain `~` and is expanded to the
home dir at load time.

## Command: `da init`

Interactive prompts via `dialoguer`, with sensible defaults:

1. host, port (default `22`), user
2. auth type: `key` or `password`
   - key → path to `.pem`
   - password → hidden input → **warn:** "senha será salva em texto puro
     em `.da/config.toml`"
3. build command, output_dir (default `dist`)
4. remote_dir
5. clean_remote? (default `yes`)

Then:
- Write `.da/config.toml`.
- Ensure `.gitignore` contains `.da/` (avoid committing secrets); create
  or append if missing.
- If config already exists, confirm overwrite before writing.

## Command: `da deploy`

Args: `--dry-run` (bool). Global flag: `-v/--verbose`.

Pipeline (each step logged via `tracing` at INFO, `passo N/8`):

1. Load `.da/config.toml`. Missing/invalid → clear stderr error +
   exit 1, suggest `da init`.
2. Delete `output_dir` locally (clear stale build).
3. Run `build.command` in local shell; abort if exit code != 0
   (SSH never opened on build failure).
4. Verify `output_dir` exists after build; error if not.
5. Open SSH session (key or password auth).
6. If `clean_remote`: delete contents inside `remote_dir` on the VPS.
7. Create `remote_dir` if it does not exist (`mkdir -p` semantics).
8. Recursively upload `output_dir` → `remote_dir` (indicatif progress
   bar, one tick per file).
9. Close session; print `deploy ok` to stdout.

### `--dry-run` behavior

Logs what *would* happen, executes nothing destructive:
- Does NOT delete local build, does NOT run build, does NOT delete
  remote, does NOT upload.
- Shows: the build command that would run, the `remote_dir` that would
  be cleaned, and the list of files under `output_dir` that would be
  uploaded (if the folder currently exists).
- Fast and safe — for verifying config before a real run.

## Logging

- `tracing` + `tracing-subscriber`, output to **stderr** (stdout stays
  clean for scripting).
- Format: `[HH:MM:SS LEVEL] message`, e.g.
  `[12:01:03 INFO] passo 3/8: rodando build 'npm run build'`.
- Default level INFO; `-v/--verbose` raises to DEBUG (ssh2 detail, file
  lists).
- **Password is never logged.** Auth logs only its type (`key` /
  `password`) and, for key, the key path.

## Error Handling & Edge Cases

- Config absent/invalid → stderr + exit 1, suggest `da init`.
- Build fails → abort before opening SSH.
- `output_dir` missing after build → error.
- Auth failure / host unreachable → clear message.
- `key_path` with `~` → expanded to home dir.
- **Safety:** `clean_remote` deletes only *inside* `remote_dir`, never
  the dir itself. Validate `remote_dir` is non-empty and not `/` before
  any delete (guard against wiping `/`).
- All errors to stderr; non-zero exit on any failure (`main() ->
  anyhow::Result<()>` + explicit exit codes where needed).

## Testing

- `config.rs`: round-trip serialize/deserialize for both auth variants;
  `~` expansion; missing-file error.
- `ssh.rs`: `remote_dir` safety validation (rejects `/`, empty); path
  join logic for upload targets. Live SSH behind an ignored integration
  test (needs a real host).
- `deploy.rs`: dry-run plan generation (lists correct files, executes
  nothing) — inject a fake transport / use a trait boundary so the
  pipeline is testable without a live host.
