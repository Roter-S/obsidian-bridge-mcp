# Shared vault configuration

Run `obm setup` to configure one or more vaults. Enter `NAME=/absolute/path`
for each vault, then a blank line to finish. Each path must be an existing
Obsidian vault containing a `.obsidian` directory. Names must be unique.
Setup validates all entries before saving and asks before replacing an existing
configuration. Declining, invalid input, or end-of-input leaves it unchanged.

The shared JSON file is located at
`$XDG_CONFIG_HOME/obsidian-bridge-mcp/config.json`, or
`$HOME/.config/obsidian-bridge-mcp/config.json` when XDG_CONFIG_HOME is unset.
Use `obm --config /path/config.json setup` for an explicit location.

```json
{
  "vaults": [
    { "name": "Notes", "path": "/absolute/path/to/Notes" }
  ]
}
```

Run `obm doctor` to validate configuration and vaults. Errors return a nonzero
exit code with repair guidance. For a Homebrew-managed installation, use
`brew update && brew upgrade obsidian-bridge-mcp`; configuration lives outside the binary and
survives upgrades. This command does not install or upgrade anything itself.
Explicit `--vault` options also override stored configuration for doctor: only
those vaults are diagnosed, and the configuration file is not loaded or changed.

Bare `obm` serves MCP over stdio using the saved configuration, never prompting.
Missing configuration reports a stderr error directing you to setup. Explicit
`--vault NAME=/absolute/path` options override the entire saved configuration;
they are never merged. The original `obsidian-mcp-rs` executable supports the
same commands and keeps existing explicit-vault MCP configurations working.

Setup also accepts repeated `--vault` options instead of reading vault lines;
replacement still requires confirmation. Prompts are only used by setup.
If an interrupted disk write leaves `config.json.pending`, inspect that file
before removing it and retrying setup; the previous configuration stays intact.
