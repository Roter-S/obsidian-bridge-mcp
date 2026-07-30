# Repository Instructions

## Shape

- Single Rust binary crate, not a workspace. The binary name is `obsidian-mcp-rs`.
- `src/main.rs` is the runtime entrypoint: parse required `--vault NAME=/absolute/path` flags, build `VaultRegistry`, then serve MCP over stdio.
- `src/tools.rs` owns the MCP tool surface and request/response schemas; `src/vault.rs` owns all vault filesystem access and path safety.

## Commands

- Build release binary for OpenCode MCP config: `cargo build --release`.
- Normal verification from README: `cargo fmt`, `cargo test`, then `cargo clippy --all-targets --all-features -- -D warnings`.
- List tests without running them: `cargo test -- --list`.
- Run one test by filter, for example: `cargo test vault::tests::search_notes_returns_matches`.

## MCP Behavior To Preserve

- The server only talks over stdio via `rmcp::transport::stdio`; tracing is intentionally sent to stderr so stdout stays protocol-safe.
- Every configured vault path must be absolute, exist, be a directory, and contain `.obsidian/`.
- Note paths are vault-relative and must end in `.md`; absolute paths, `..`, `.obsidian`, and symlinks are rejected.
- `list_notes` defaults to 100 and clamps at 500; `search_notes` defaults to 20 and clamps at 200.
- `create_note` creates parent folders but fails if the note exists; `append_note` and `update_note` require an existing note.

## Testing Notes

- Unit tests live inside `src/cli.rs` and `src/vault.rs`; async vault tests use `tempfile` vaults with a synthetic `.obsidian/` directory.
