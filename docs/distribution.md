# Install and update prebuilt binaries

Obsidian Bridge MCP provides native `obm` binaries for macOS and GNU/Linux on
ARM64 and x86_64. Running a prebuilt binary does not require Rust, Node, or Python.

## Install with Homebrew

```sh
brew install roter-s/tap/obsidian-bridge-mcp
```

The package is named `obsidian-bridge-mcp`; the command you run is `obm`.
Homebrew installs the executable but does not configure vaults or change your
MCP client settings.

After the first installation:

```sh
obm setup
obm doctor
```

Register `obm` as a stdio MCP server in your client. See the
[README](../README.md#mcp-client-configuration-pi-opencode-claude-cursor-etc)
and [configuration guide](configuration.md).

## Update with Homebrew

```sh
brew update
brew upgrade roter-s/tap/obsidian-bridge-mcp
brew list --versions obsidian-bridge-mcp
obm doctor
```

Upgrading replaces the executable, not your shared vault configuration. Do not
run setup again unless you want to change that configuration.

Reconnect or restart the MCP server in each client after an upgrade. Already
running processes continue using the previous binary and clients may cache tool
schemas and connection instructions.

If a client still uses an old version, check which executable it launches:

```sh
command -v obm
brew --prefix obsidian-bridge-mcp
```

A client configured with a repository's `target/release/obm` will not be updated
by Homebrew. Point it at the Homebrew-installed executable if you want Homebrew
to manage its upgrades.

## Download an archive manually

Download the appropriate archive and matching `.sha256` file from the
[GitHub releases page](https://github.com/Roter-S/obsidian-bridge-mcp/releases).
Use the files attached to the same release; do not substitute workflow artifacts
or archives from another version.

| System | Archive target |
| --- | --- |
| macOS, Apple Silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |
| GNU/Linux, ARM64 | `aarch64-unknown-linux-gnu` |
| GNU/Linux, x86_64 | `x86_64-unknown-linux-gnu` |

Archive names follow `obm-VERSION-TARGET.tar.gz` and contain an executable named
`obm`. Linux downloads target GNU/Linux, not musl-based distributions such as
Alpine. Signing and notarization are not provided.

For example, in a directory containing both downloaded files:

```sh
# macOS: verify the checksum before extracting.
shasum -a 256 -c obm-VERSION-aarch64-apple-darwin.tar.gz.sha256

# GNU/Linux: use sha256sum instead.
sha256sum -c obm-VERSION-aarch64-unknown-linux-gnu.tar.gz.sha256
```

Replace `VERSION` and the target with the files you downloaded; run only the
command for your platform. If checksum verification fails, do not install the
archive. Download both files again from the release page.

Extract the verified archive and place `obm` in a directory on your `PATH`, or
configure your MCP client with its absolute executable path. Manual installations
must be updated manually; Homebrew does not manage them.

## Build from source

With a Rust toolchain installed, run from the repository:

```sh
cargo install --path . --bin obm --locked
```

Then run `obm setup` if you have not configured vaults yet, and `obm doctor` to
validate them. Reconnect your MCP clients after replacing the executable.

## Check behavior changes when upgrading

Read the release notes before upgrading clients that process tool output directly.
Starting with v0.3.0:

- `read_note` returns at most 200 lines by default and clamps explicit limits to
  500 lines. Follow `next_offset` while `has_more` is true to read more.
- `list_tags` returns the first 100 tags by default, with a maximum of 500 per page.
- Lists, searches, backlinks, and tags provide pagination metadata. Their offsets
  are zero-based; read offsets are 1-based line numbers.
- Path-only search matches omit the redundant `excerpt` field. Unused write-result
  fields are omitted rather than returned as `null`.
- Read failures in searches, backlinks, and tag scans are reported instead of
  silently returning incomplete successful results.

See the [usage guide](usage.md) for examples and troubleshooting.
