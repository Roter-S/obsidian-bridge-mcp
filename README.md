# Obsidian Bridge MCP

Rust MCP server for connecting Obsidian vaults to AI assistants (Pi, OpenCode, Claude, Cursor, Copilot, etc.).

## Features

- Fast, native binary for macOS and Linux (ARM64 and x86_64)
- Shared vault configuration across all AI agents (`obm setup`)
- Diagnostic tool to inspect vault setup and upgrades (`obm doctor`)
- Multiple configured vaults
- Safe markdown-only note access
- MCP tools:
  - `list_vaults`
  - `list_notes`
  - `read_note`
  - `search_notes`
  - `create_note`
  - `append_note`
  - `update_note`

## Installation

### Homebrew (Recommended)

```bash
brew install roter-s/tap/obsidian-bridge-mcp
```

### From Source

```bash
cargo install --path .
```

## Quick Start

1. **Configure your vaults once:**

   ```bash
   obm setup
   ```

   Follow the prompts to add your vault paths (e.g. `The Core=/Users/.../Documents/Obsidian/The Core`). Vaults are saved to `~/.config/obsidian-bridge-mcp/config.json`.

2. **Verify configuration:**

   ```bash
   obm doctor
   ```

## MCP Client Configuration (Pi, OpenCode, Claude, Cursor, etc.)

Because `obm` reads your shared configuration, your AI agents only need to execute `obm` directly without repeating `--vault` flags:

```json
{
  "mcp": {
    "obsidian": {
      "type": "local",
      "command": ["obm"],
      "enabled": true
    }
  }
}
```

Pi reads `~/.config/mcp/mcp.json` automatically through `pi-mcp-adapter`. After changing that file, run `/reload` (or restart Pi). The configured vault must be the vault root, not the project folder. For Nexus Core, use vault `The Core` and folder `Projects/Nexus Core`.

> **Note**: You can still provide explicit vaults via `--vault NAME=/path` if you want to override the saved configuration for a specific agent.

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
