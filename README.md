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
  - `list_notes` (compact paths by default, optional `details` and `tag` filter)
  - `read_note` (200 lines by default; paginated with `offset` and `limit`)
  - `get_note_outline` (headings/TOC with line numbers)
  - `patch_note` (token-efficient find & replace)
  - `search_notes` (ordered streaming search with early stop, ignoring case and accents; optional `path_only`)
  - `get_backlinks` (find wikilinks `[[...]]` pointing to a note)
  - `list_tags` (all `#tags` and frontmatter tags)
  - `create_note` (atomic write)
  - `append_note`
  - `update_note` (atomic write; optional `expected_total_lines` guard)
  - `move_note` (move or rename; never replaces an existing note)
  - `delete_note` (moves the note to the vault's `.trash`)

## Performance

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/benchmark-dark.svg">
  <img src="docs/assets/benchmark-light.svg" alt="Bar chart of median request times on a synthetic vault of 20,000 generated notes: paged requests take about 1 ms, deeper pages take 11 to 16 ms, and reading the whole vault takes 40 to 1,056 ms." width="800">
</picture>

A request for one page of results stops reading as soon as the page is full, so it takes about a millisecond at any page size. A page deep in the results, or one whose matches are spread thinly, reads more notes before it fills. Reading everything costs a pass over the vault, and following `next_offset` to the end repeats part of that pass on every page.

| Request | 2,000 notes | 20,000 notes |
| --- | ---: | ---: |
| List 100 notes | 0.23 ms | 0.84 ms |
| List 500 notes (largest page) | 0.64 ms | 0.93 ms |
| Search paths, 200 matches | 0.34 ms | 0.82 ms |
| Search content, 20 matches | 0.20 ms | 0.81 ms |
| Search content, 200 matches (largest page) | 0.50 ms | 1.10 ms |
| List 100 notes from the middle of the vault | 1.2 ms | 11 ms |
| Backlinks, 50 matches (1 note in 20 links to the target) | 13 ms | 14 ms |
| Search content, 200 matches starting at match 10,000 | 15 ms | 16 ms |
| Newest 500 notes with dates | 3.3 ms | 40 ms |
| Count every tag | 18 ms | 167 ms |
| Every match of a rare phrase, typed without its accent | 28 ms | 391 ms |
| Search content, none found | 28 ms | 392 ms |
| List every note, 500 per page | 5.3 ms | 433 ms |
| Every backlink, 200 per page | 26 ms | 1,056 ms |

Reading 200 lines from a 100,000-line note takes 3.4 ms; reading 500 lines starting at line 90,000 takes 3.5 ms.

These are medians of nine runs on an Apple M3 Pro against temporary vaults that the benchmark generates (20 folders, notes of 12 to 51 lines, a third of the lines accented; 63,120 and 631,200 lines in total) with a warm filesystem cache. They are not measurements of a real vault. They time the vault operations inside the server, not a cold disk or a full MCP round trip through a client, and they are not a comparison with other servers. Reproduce them with:

```bash
cargo test --locked --release vault::benchmarks::synthetic_vault -- --ignored --nocapture
```

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

Because `obm` reads your shared configuration, each agent only needs to register `obm` as a **stdio MCP server**, without repeating `--vault` flags. No Rust runtime, Node, Python, skill, or agent-specific wrapper is required to run the installed binary.

Each client has its own configuration format. For example, **OpenCode** uses:

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

For Pi, Codex, or another client, use that client's MCP configuration format with executable `obm` (or its absolute installed path), no arguments, and stdio transport. Restart/reconnect the MCP server after upgrading so the client refreshes instructions and tool schemas. The configured filesystem path must be the vault root, not a project folder.

> **Note**: You can still provide explicit vaults via `--vault NAME=/path` if you want to override the saved configuration for a specific agent.

## AGENTS.md Convention

The MCP server is self-describing: its initialization instructions include configured vault names, and tool schemas explain browsing, searching, paging, and editing. **A skill is not required** for basic operation in any MCP client.

An optional `AGENTS.md` records project-specific routing that the server cannot infer safely. The MCP server does not read it; the agent reads it and passes the right `vault`, `folder`, and `path`. This avoids repeated cross-vault discovery.

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

> **Note**: `vault` is optional across all tools whenever a single vault is configured. With multiple vaults, pass an exact configured name; the server never guesses.

### Efficient agent workflow

1. Use the vault names supplied at MCP initialization; call `list_vaults` if the client does not expose server instructions.
2. Browse a folder with `list_notes`. Do not use `search_notes` with `query: ""`.
3. Search with a **non-empty literal** query; use `path_only: true` when looking for filenames/paths without reading note bodies.
4. Read only relevant lines. `read_note` defaults to 200 lines and clamps `limit` to 1–500. Use outlines when structure is useful, not automatically before every small read.
5. Prefer `patch_note` for a small unique edit, `append_note` for additions, and `update_note` only for an intentional full replacement, with `expected_total_lines` from your last read.
6. `delete_note` moves a note to `.trash`; `move_note` renames or restores it.

`list_notes`, `search_notes`, `get_backlinks`, and `list_tags` return `count` (this page), `has_more`, and `next_offset` when more results exist. Pass that offset with the same filters to continue. Their offsets are **zero-based result counts**. `read_note` uses **1-based line offsets** and also returns `has_more` / `next_offset`. Do not fetch more pages unless the task needs them.

See the [usage guide](docs/usage.md) for tool examples, pagination, targeted edits, and troubleshooting.

`list_vaults`

```json
{}
```

`list_notes`

```json
{
  "vault": "The Core",
  "folder": "Projects/nexus-core-backend",
  "limit": 100,
  "details": false
}
```

`read_note`

```json
{
  "vault": "The Core",
  "path": "Projects/nexus-core-backend/00-Overview.md",
  "offset": 1,
  "limit": 50
}
```

`get_note_outline`

```json
{
  "vault": "The Core",
  "path": "Projects/nexus-core-backend/00-Overview.md"
}
```

`patch_note`

```json
{
  "vault": "The Core",
  "path": "Projects/nexus-core-backend/00-Overview.md",
  "old_text": "status: draft",
  "new_text": "status: active"
}
```

`search_notes`

```json
{
  "vault": "The Core",
  "folder": "Projects/nexus-core-backend",
  "query": "pendiente",
  "limit": 20,
  "path_only": false
}
```

`get_backlinks`

```json
{
  "vault": "The Core",
  "path": "00-Overview.md",
  "limit": 50
}
```

`list_tags`

```json
{
  "vault": "The Core"
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
