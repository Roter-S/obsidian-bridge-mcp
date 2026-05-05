# obsidian-mcp-rs

Rust MCP server for working directly with Obsidian vaults stored on disk.

## Features

- Multiple configured vaults
- Safe markdown-only note access
- `list_vaults`
- `list_notes`
- `read_note`
- `search_notes`
- `create_note`
- `append_note`
- `update_note`

## Install

```bash
cargo build --release
```

## Run

```bash
obsidian-mcp-rs \
  --vault "The Core=/Users/roberto/Library/Mobile Documents/iCloud~md~obsidian/Documents/The Core" \
  --vault "BYTE=/Users/roberto/Library/Mobile Documents/iCloud~md~obsidian/Documents/BYTE"
```

Each `--vault` must use `NAME=/absolute/path` and the target directory must contain `.obsidian/`.

## OpenCode Configuration

```json
{
  "mcp": {
    "obsidian": {
      "type": "local",
      "command": [
        "/Users/roberto/Web/obsidian-mcp-rs/target/release/obsidian-mcp-rs",
        "--vault",
        "The Core=/Users/roberto/Library/Mobile Documents/iCloud~md~obsidian/Documents/The Core",
        "--vault",
        "BYTE=/Users/roberto/Library/Mobile Documents/iCloud~md~obsidian/Documents/BYTE"
      ],
      "enabled": true
    }
  }
}
```

## AGENTS.md Convention

The MCP server does not read `AGENTS.md`. OpenCode should read project instructions and pass the right `vault`, `folder`, and `path` values to the MCP tools.

Example:

```md
## Obsidian Project Memory

Use the `obsidian` MCP server for project memory.

- Vault: `The Core`
- Project directory: `Projects/nexus-core-backend`
- Project index: `Projects/nexus-core-backend/00-Overview.md`

For project plan questions, read and search inside `Projects/nexus-core-backend`.
For progress updates, append to `Projects/nexus-core-backend/00-Overview.md`.
```

## Tool Inputs

`list_vaults`

```json
{}
```

`list_notes`

```json
{
  "vault": "The Core",
  "folder": "Projects/nexus-core-backend",
  "limit": 100
}
```

`read_note`

```json
{
  "vault": "The Core",
  "path": "Projects/nexus-core-backend/00-Overview.md"
}
```

`search_notes`

```json
{
  "vault": "The Core",
  "folder": "Projects/nexus-core-backend",
  "query": "pendiente",
  "limit": 20
}
```

`create_note`, `append_note`, `update_note`

```json
{
  "vault": "The Core",
  "path": "Projects/nexus-core-backend/Bitacora.md",
  "content": "# Bitacora\n"
}
```

## Safety Rules

- Only configured vaults are accessible.
- Note paths must be relative and end with `.md`.
- `.obsidian` access is blocked.
- Absolute paths and `..` are blocked.
- Symlinks are rejected.
- `create_note` fails if the file already exists.
- `append_note` and `update_note` require the file to exist.

## Development

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
