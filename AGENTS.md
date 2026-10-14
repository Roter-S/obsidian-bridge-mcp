# Repository Instructions

## Project Principles

- Keep implementation, tests, and distribution tooling in Rust. Do not introduce Python, Node, or shell-script dependencies.
- The MCP server must be self-describing and usable from any compatible client (Pi, Codex, etc.). Do not add mandatory skills or agent-specific wrappers.
- Put usage guidance in MCP initialization instructions, tool descriptions, schemas, and actionable errors. Optional project instructions may identify a vault/folder, but must not be needed to understand the tools.
- Prefer compact responses and bounded work. Do not claim token savings or throughput improvements without measurements.

## Shape

- Single Rust package, not a workspace. Public product name: Obsidian Bridge MCP; primary executable: `obm`.
- `src/bin/obm.rs` and the compatibility entrypoint `src/main.rs` (`obsidian-mcp-rs`) both call the shared runtime exported by `src/lib.rs`.
- `src/app.rs` dispatches setup/doctor/server mode and configures stderr tracing; `src/server.rs` serves MCP over stdio.
- `src/cli.rs` parses commands and vault flags; `src/config.rs` resolves, loads, and validates shared configuration; `src/setup.rs` and `src/doctor.rs` own their respective commands.
- `src/tools.rs` owns MCP instructions, tool annotations, request/response schemas, and response pagination.
- `src/vault.rs` owns note filesystem access, path safety, searching, and writes; `src/error.rs` maps errors to MCP errors.
- `src/bin/dist.rs` provides Rust release packaging and Homebrew formula generation.

## Commands

- Build release binaries: `cargo build --release`.
- Run the primary executable explicitly: `cargo run --bin obm -- doctor` or `cargo run --bin obm -- setup`.
- Verify changes: `cargo fmt --check`, `cargo test`, then `cargo clippy --all-targets --all-features -- -D warnings`. Use `cargo fmt` to apply formatting.
- List tests: `cargo test -- --list`.
- Run a filtered test: `cargo test vault::tests::search_notes_returns_matches`.
- Run the public MCP wire-contract tests: `cargo test --test mcp_contract`.

## Configuration and Transport

- Without explicit vault flags, server mode loads `obsidian-bridge-mcp/config.json` under `XDG_CONFIG_HOME`, or `~/.config` when that variable is unset. `--config PATH` overrides the config location.
- Explicit `--vault NAME=/absolute/path` flags replace stored vault configuration for that invocation. A bare absolute vault path is also accepted and its final folder name supplies the vault name.
- Every vault path must be absolute, exist, be a directory, and contain `.obsidian/`.
- Server stdout is exclusively MCP protocol traffic via `rmcp::transport::stdio`; tracing belongs on stderr. Setup/doctor may use stdout in their own modes.
- MCP initialization includes configured vault names and the version from `CARGO_PKG_VERSION`. Keep descriptions and schemas aligned with runtime behavior.
- Each client has its own MCP process-registration format; do not present one client's configuration as universal.

## MCP Contracts to Preserve

- `vault` is optional only when exactly one vault is configured. With multiple vaults, require an exact name; never choose a vault arbitrarily, especially for writes.
- Note paths are vault-relative and must end in `.md`. Reject absolute paths, `..`, `.obsidian`, and symlinks. Omit optional `folder` to select the vault root.
- Use `list_notes` for browsing without a query. `search_notes` requires non-empty literal text after trimming; reject it with an actionable query error, not `EmptyPath`.
- `path_only` search must not read note bodies or return a duplicate path as an excerpt.
- Default / maximum limits: `list_notes` 100/500; `search_notes` 20/200; `get_backlinks` 50/200; `list_tags` 100/500; `read_note` 200/500 lines. MCP limits clamp to at least 1.
- Lists, searches, backlinks, and tags use zero-based result offsets. `read_note` uses 1-based line offsets. Return `has_more` and `next_offset` when more results remain; `count` means the current page size, not a total.
- Search/backlinks are ordered by path then line; compact lists by path; detailed lists by modification time descending then path; tags by usage descending then tag. Preserve stable ordering for unchanged vault contents.
- Search/backlink excerpts are capped at 240 Unicode characters plus an ellipsis when shortened. Omit unused optional response fields instead of emitting redundant nulls.
- Preserve standard read-only/destructive/closed-world tool annotations. They inform clients but do not replace client approval policies.
- Missing notes/folders are resource-not-found errors. Do not silently discard read failures and return an apparently complete successful result.

## Filesystem Work and Writes

- Search/backlinks stream lines and stop after filling the requested page (including pagination lookahead). Avoid unbounded per-file tasks or accumulated match vectors.
- Range reads retain selected lines rather than the full note; they still scan to count total lines. Tag extraction has at most 32 active tasks.
- Line limits are not byte/token limits. Do not silently discard note content to claim a hard output ceiling.
- `create_note` creates parent folders and must fail if the destination exists, including under concurrent creation. Preserve atomic no-replace publication; an existence check followed by overwrite-capable rename is insufficient.
- `append_note`, `update_note`, and `patch_note` require an existing note. A patch requires exactly one matching snippet. Updates and patches use atomic replacement.
- Atomic writes are not transactions or version checks. Do not claim protection from conflicting external edits or hostile filesystem races.

## Testing

- Unit tests live in `src/cli.rs`, `src/tools.rs`, and `src/vault.rs`; vault tests use temporary vaults with synthetic `.obsidian/` directories.
- `tests/cli_config.rs` covers setup, shared configuration, and doctor; `tests/distribution.rs` covers packaging and formula generation.
- `tests/mcp_contract.rs` starts the real `obm` binary and exchanges JSON-RPC over stdin/stdout, without an agent SDK or skill.
- For tool contract changes, update schemas/descriptions, wire tests, and user documentation together. Cover errors, pagination boundaries, compact output, and path safety; preserve concurrent-create and bounded-work regressions.
- Tests must not depend on personal vaults or alter installed client/Homebrew configuration. Source changes and local builds do not update installed binaries; do not publish/install unless requested.

## Documentation

- `README.md` and `docs/` are user-facing: installation, configuration, tool examples, pagination, compatibility, and troubleshooting.
- `docs/usage.md` is the tool usage guide; `docs/configuration.md` and `docs/distribution.md` cover configuration and distribution.
- Do not put internal audits, development tracking, session reports, or test-run summaries in `docs/`. Keep repository contributor rules in this file and internal session bookkeeping out of user guides.
- Preserve the existing English language of repository documentation unless asked otherwise. Keep links valid and behavior changes visible to users.
