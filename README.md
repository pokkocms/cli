# Pokko CLI

Manage [Pokko](https://pokko.io) CMS content model schemas as code — a Terraform-style workflow for your content models.

```
pokko pull        # fetch remote schemas to local YAML files
pokko plan        # diff local changes against remote state
pokko apply <plan> # apply a saved plan to the API
```

## Installation

### From Git (recommended)

Requires [Rust](https://rustup.rs) 1.70+.

```sh
cargo install --git https://github.com/pokko/cli
```

To update to the latest version, run the same command again.

### Pre-built binaries

Download the latest binary for your platform from the [Releases](../../releases) page.

| Platform | File |
|----------|------|
| macOS (Apple Silicon) | `pokko-aarch64-apple-darwin` |
| macOS (Intel) | `pokko-x86_64-apple-darwin` |
| Linux (x86_64) | `pokko-x86_64-unknown-linux-gnu` |
| Linux (ARM64) | `pokko-aarch64-unknown-linux-gnu` |
| Windows | `pokko-x86_64-pc-windows-msvc.exe` |

Make the binary executable and move it somewhere on your `PATH`:

```sh
chmod +x pokko-aarch64-apple-darwin
mv pokko-aarch64-apple-darwin /usr/local/bin/pokko
```

### From source

```sh
git clone https://github.com/pokko/cli
cd cli
cargo build --release
# binary is at ./target/release/pokko
```

## Getting started

### 1. Authenticate

```sh
pokko login
```

Opens your browser to complete authentication. Your token is stored in `~/.pokko/credentials.toml`.

### 2. Initialise a project

Run this in the directory where you want to manage your schemas:

```sh
pokko init
```

This walks you through selecting a project and environment, then creates:
- `.pokko/config.toml` — project config
- `models/` — where YAML schema files live
- `.pokko/plans/` — saved plan files

### 3. Pull current schemas

```sh
pokko pull
```

Writes one YAML file per model into `models/`.

### 4. Make changes

Edit the YAML files. Each file represents one content model:

```yaml
name: Blog Post
alias: blog_post
usage: ENTRY
fields:
  - alias: title
    name: Title
    type: SCALAR
    config:
      type: text
    required: true
    multi: false
```

### 5. Preview changes

```sh
pokko plan
```

Shows a diff of what will change and saves a plan file to `.pokko/plans/`.

### 6. Apply changes

```sh
pokko apply .pokko/plans/<plan-file>.json
```

The apply step re-verifies both your local files and the remote state match what was captured at plan time. If anything has changed in between, the apply is aborted — re-run `pokko plan` to get a fresh plan.

Use `--force` to apply plans that contain destructive changes (model or field deletions).

## Commands

| Command | Description |
|---------|-------------|
| `pokko login` | Authenticate via browser |
| `pokko logout` | Remove stored credentials |
| `pokko whoami` | Show current auth status |
| `pokko init` | Scaffold config and models directory |
| `pokko pull` | Fetch remote schemas to local YAML |
| `pokko status` | Show pending diff without saving a plan |
| `pokko plan` | Diff and save a plan file |
| `pokko apply <plan>` | Apply a saved plan |

## Configuration

`.pokko/config.toml` (created by `pokko init`):

```toml
api_url = "https://au-syd1.pokko.io/graphql"
project = "<project-uuid>"
environment = "<environment-uuid>"
models_dir = "models"
```

All values can be overridden with flags or environment variables:

| Flag | Env var | Description |
|------|---------|-------------|
| `--api-url` | `POKKO_API_URL` | API endpoint |
| `--token` | `POKKO_TOKEN` | Auth token (skips credential file) |
| `--project` | `POKKO_PROJECT` | Project UUID |
| `--environment` | `POKKO_ENVIRONMENT` | Environment UUID |

## CI/CD usage

Set `POKKO_TOKEN`, `POKKO_PROJECT`, and `POKKO_ENVIRONMENT` as environment secrets, then:

```sh
pokko plan --out plan.json
pokko apply plan.json --force
```

## Gitignore

Add these to your `.gitignore` to avoid committing generated or sensitive files:

```
.pokko/
models/
```

Commit only the files you explicitly want to track.

## License

MIT
