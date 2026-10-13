# Prepared binary distribution

This is local scaffolding, not an announcement of available releases or a tap.
The Homebrew package is `obsidian-bridge-mcp`, with formula class
`ObsidianBridgeMcp` and filename `obsidian-bridge-mcp.rb`. Its executable is
`obm`; the legacy `obsidian-mcp-rs` executable remains independent. Release packaging requires the `obm` entrypoint to exist in the
selected version tag. No README changes are part of this distribution work.

## Prepare archives

A maintainer may manually dispatch **Prepare versioned archives** with an
**existing** tag such as `v0.1.0`, only when that tag matches the crate version.
Checkout uses the explicit tag reference, not main. Tags must use
`vMAJOR.MINOR.PATCH` (no prerelease suffix). Main commits do not trigger releases.
The workflow has read-only repository permissions and uploads workflow artifacts;
it does not create or publish a GitHub release.

| Native runner | Archive target |
| --- | --- |
| macos-14 | aarch64-apple-darwin |
| macos-15-intel | x86_64-apple-darwin |
| ubuntu-24.04-arm | aarch64-unknown-linux-gnu |
| ubuntu-24.04 | x86_64-unknown-linux-gnu |

Each job tests and builds on the matching native runner, then packages `obm` at
the archive root with executable permissions and a SHA256 sidecar. Runner
availability depends on GitHub and repository eligibility. Workflow YAML alone
**does not verify platform support**. Linux archives target GNU/Linux, not musl;
minimum OS/libc compatibility and executable runtime behavior need actual
platform validation. Local fixture packaging was exercised on macOS ARM64;
that does not validate the Rust binary or any other platform.

For local packaging (Python 3.11+ and Bash required):

```sh
bash scripts/package-release.sh v0.1.0 aarch64-apple-darwin target/release/obm dist
```

The script rejects version mismatches, non-native targets, non-executable input,
and existing artifact destinations. Version is read from `Cargo.toml`; an
optional fifth argument selects a manifest for isolated testing. Successful
packaging is silent. It does not infer executable architecture from the file:
the caller must supply the native build, as the workflow does.

## Render a formula only from real artifacts

Collect all four archives and their `.tar.gz.sha256` sidecars in one directory.
The sidecar format is `SHA256`, two spaces, archive basename, then newline.
The renderer checks each checksum against the actual archive bytes before
writing anything; missing or mismatched inputs leave the destination unchanged.

```sh
python3 scripts/render-formula.py 0.1.0 dist obsidian-bridge-mcp.rb
```

The generated formula selects a binary archive for macOS/Linux ARM64/x86_64
from `Roter-S/obsidian-bridge-mcp` GitHub releases. It installs only `obm`:
no setup command, vault discovery, or external configuration writes occur during
Homebrew installation. These URLs describe the intended publication layout,
not assets currently available. Checksums are derived from real files, never
placeholders. The renderer checks integrity, not archive authenticity or runtime
compatibility; use only reviewed artifacts from the intended workflow run.

## Publication is a separate maintainer operation

Before publishing, inspect all four job results, verify checksums and runtime
behavior on supported systems, and explicitly authorize a versioned GitHub
release. Upload the exact archives and sidecars to that release. Only after
those URLs resolve should a maintainer publish the generated
`obsidian-bridge-mcp.rb` in the proposed `roter-s/tap` Homebrew tap, after separate
explicit authorization. Neither this workflow nor these scripts create
a remote tap or publish a release. Signing/notarization is not implemented.

The intended tap-qualified package is **PROPOSED**, not currently available:
`roter-s/tap/obsidian-bridge-mcp`. Only after the tap and release are published,
the proposed user commands are:

```sh
brew install roter-s/tap/obsidian-bridge-mcp
brew upgrade roter-s/tap/obsidian-bridge-mcp
```

The package name is `obsidian-bridge-mcp`; users still invoke `obm` after
installation. Binary installation needs no Rust toolchain. Upgrades replace the executable,
not configuration stored outside Homebrew. Users run setup separately after
installation when desired. No source-build fallback is promised here.

## Local checks

```sh
python3 -m unittest discover -s tests -p test_distribution.py -v
bash -n scripts/package-release.sh
```

Tests use isolated temporary files and a small executable fixture; they do not
install software, write user configuration, or publish anything.
