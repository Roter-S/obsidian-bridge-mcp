# Using the Obsidian MCP server

Use these tool inputs with any MCP-compatible client, including Pi and Codex. Register `obm` as a stdio MCP server using your client's configuration format. No skill or agent-specific wrapper is required.

For installation and vault configuration, see the [README](../README.md#quick-start).

## Choose a vault

The server provides configured vault names in its connection instructions. If your client does not show those instructions, call `list_vaults`:

```json
{}
```

With one configured vault, you can omit `vault`. With multiple vaults, pass an exact name, including spaces and capitalization:

```json
{
  "vault": "The Core",
  "folder": "Projects/SZAPETA"
}
```

Use the vault root when configuring `obm`. Pass project folders as vault-relative `folder` values, not as separate vaults.

An optional project `AGENTS.md` can tell your agent which vault and folder to use. The MCP server itself does not read that file.

## Browse a folder

Call `list_notes` when you want to discover notes without a search term:

```json
{
  "vault": "The Core",
  "folder": "Projects/SZAPETA",
  "limit": 20
}
```

The list includes notes in subfolders. Omit `folder` to browse the whole vault. Compact note paths are returned by default; add `details: true` only when you need titles, modification dates, and sizes.

**Do not use `search_notes` with an empty query to list a folder.**

## Search for notes or text

Call `search_notes` with a non-empty, case-insensitive literal query. Queries are not regular expressions.

To find text in note contents:

```json
{
  "vault": "The Core",
  "folder": "Projects/SZAPETA",
  "query": "pending",
  "limit": 10
}
```

To find filenames or paths without reading note contents:

```json
{
  "vault": "The Core",
  "folder": "Projects/SZAPETA",
  "query": "Checklist",
  "path_only": true,
  "limit": 10
}
```

Content matches include a path, a 1-based line number, and a preview of up to 240 characters plus an ellipsis if shortened. A preview may omit text near the end of a long line; read the original note at the returned line when needed. Path-only matches omit the preview.

## Read only what you need

Call `read_note` with a vault-relative `.md` path:

```json
{
  "vault": "The Core",
  "path": "Projects/SZAPETA/Overview.md",
  "offset": 1,
  "limit": 50
}
```

Without `limit`, the tool returns up to **200 lines**, not necessarily the entire note. The maximum is **500 lines per call**. The response includes `total_lines`, `lines_read`, `has_more`, and `next_offset` when more lines remain.

Use `get_note_outline` when you need headings and their line numbers before choosing a section to read:

```json
{
  "vault": "The Core",
  "path": "Projects/SZAPETA/Overview.md"
}
```

An outline returns headings instead of the body. You do not need to request an outline before every small read.

## Continue to another page

| Tool | Default limit | Maximum limit | `offset` means | Result order |
| --- | ---: | ---: | --- | --- |
| `list_notes` | 100 | 500 | Zero-based number of notes to skip | Path; with `details: true`, newest modification first, then path |
| `search_notes` | 20 | 200 | Zero-based number of matches to skip | Path, then line |
| `get_backlinks` | 50 | 200 | Zero-based number of matching lines to skip | Path, then line |
| `list_tags` | 100 | 500 | Zero-based number of tags to skip | Usage descending, then tag |
| `read_note` | 200 | 500 | **1-based starting line** | Document order |

Lists and searches return `count` for the current page, not the total number of results. For example, a `list_notes` response can look like:

```json
{
  "notes": ["Projects/SZAPETA/Overview.md"],
  "count": 1,
  "has_more": true,
  "next_offset": 1
}
```

To continue, call the same tool with the same filters and pass `next_offset` as `offset`. For this example:

```json
{
  "vault": "The Core",
  "folder": "Projects/SZAPETA",
  "limit": 1,
  "offset": 1
}
```

Stop when `has_more` is false, or as soon as you have enough information for your task. Do not reuse an offset with different filters. If notes change between calls, page contents can shift; restart the listing if you need to check the current results again.

Compact listings, searches, and backlinks traverse folders as needed and stop after filling the page plus one continuation result. A partial page does not validate unvisited subfolders or unread note content; later pages can report an I/O or symlink error when that part of the vault is reached. Encountered errors are not silently skipped. Detailed listings must inspect metadata across the selected folder to find the newest notes, and tag counts require a full-vault scan. Results are read from the filesystem on each request, not from a persistent index.

## Make targeted edits

Use `patch_note` for a small change instead of sending the entire document:

```json
{
  "vault": "The Core",
  "path": "Projects/SZAPETA/Overview.md",
  "old_text": "status: draft",
  "new_text": "status: active"
}
```

`old_text` must match exactly once. If it appears multiple times, include more surrounding text to identify the intended occurrence.

Choose the write tool according to the task:

- `create_note`: create a new note; fails if the destination already exists. Parent folders are created as needed.
- `append_note`: add content to the end of an existing note. Include any required newline in `content`.
- `patch_note`: replace one unique existing text snippet.
- `update_note`: intentionally replace the entire contents of an existing note.

Avoid simultaneous edits to the same note from multiple agents or Obsidian. A completed write does not guarantee that another editor has not changed the note since you last read it. Do not automatically retry an append after an uncertain connection failure: it may append the content twice.

## Find backlinks and tags

Call `get_backlinks` to find lines containing wikilinks to a note:

```json
{
  "vault": "The Core",
  "path": "Overview.md",
  "limit": 20
}
```

Results are matching lines, not necessarily distinct source notes. Backlink matching is not a complete replacement for Obsidian's link resolver; duplicate note titles and relative links can be ambiguous.

Call `list_tags` to list tags and their note counts:

```json
{
  "vault": "The Core",
  "limit": 50
}
```

Tag listing scans the vault. Reuse the returned information when it is still sufficient instead of requesting it repeatedly.

## Troubleshooting

| Message or symptom | What to do |
| --- | --- |
| `multiple vaults are configured` | Pass `vault` with an exact name from the error or `list_vaults`. |
| `unknown vault` | Check the configured name; a project folder is not a vault name. |
| `search query cannot be empty` | Supply a search term, or use `list_notes` to browse a folder. Whitespace-only queries are also rejected. |
| `folder not found` / `note not found` | Check the selected vault and the vault-relative path. Use `list_notes` to discover existing note paths. |
| `path cannot be empty` | Supply a note path or folder where required; omit the optional `folder` field to use the vault root. |
| Absolute path, traversal, or symlink rejected | Use a path inside the configured vault, without an absolute prefix or `..`. Symlinks and `.obsidian` access are not supported. |
| Note path rejected | Include the `.md` extension. |
| Patch text is missing or ambiguous | Read the relevant lines again and supply an exact, unique snippet. |
| A read or search reports an I/O error | Check that the affected note exists, is readable, and contains valid UTF-8 text. |
| A note or list appears incomplete | Inspect `has_more` and request `next_offset` if you need more. |
| The client still shows old tool descriptions or behavior after upgrading | Restart or reconnect the MCP server to refresh cached instructions and tool schemas. Verify the client is launching the intended `obm` executable. |

For configuration problems, run `obm doctor`.

## Keep requests small and safe

- Scope browsing and searches to the project folder when possible.
- Prefer path-only searches when looking for note names.
- Read relevant sections and request additional pages only when necessary.
- Use compact listings unless metadata is needed.
- Treat search snippets as previews, not complete note contents.
- A line limit is not a token limit: a single very long line can still produce a large response.
- Keep vaults in trusted local directories. Path restrictions are not a sandbox for untrusted processes that can modify those directories.
