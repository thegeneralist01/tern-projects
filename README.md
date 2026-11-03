# tern-projects

A curated project sidebar and unified agent history search for [Tern](https://stencil.so/tern).

Bring order to your AI coding sessions across **OMP, Claude Code, Codex, Pi, OpenCode, and Cursor**.

![tern-projects sidebar overview](docs/screenshots/showcase-overview.png)

---

## Quickstart

Run this inside Tern:

```sh
tern plugin install github.com/thegeneralist01/tern-projects
```

**Toggle the sidebar:**
- **macOS:** `⌘ ⌥ S`
- **Windows / Linux:** `Ctrl + Alt + S`

*(Or open **Projects** from the Tern command palette).*

Tern fetches the plugin, and the background engine builds automatically on first load. Rust/Cargo and your platform's native C build tools are required; no manual compile step is needed.

On macOS, install Apple developer tools (`xcode-select --install` if needed) or select your Xcode installation. Automatic builds use the SDK selected by `xcrun --sdk macosx` and Apple's compiler/linker, rather than mixing an unrelated `cc` from `PATH` with a different SDK.

### Updates

Projects checks the official GitHub repository at startup when its last check is at least 24 hours old, then daily while Tern is running. Checks compare stable versions in `plugin.toml`; they do not install anything automatically. Disable **Automatic checks**, or use **Check now**, under **Settings → Plugin updates**.

When a newer version is available, choose **Review…** and explicitly confirm **Install update**. Tern installs the latest official `main` branch and reloads the plugin; open Projects panes may close. The version shown is the last checked version, not a pinned download.

For a manual update, run:
```sh
tern plugin install github.com/thegeneralist01/tern-projects --force
```

**Copy command** in Plugin updates copies that same command, including when a network check or installation fails. Tern's installer replaces plugin files, not your project library or plugin data; the history index remains a rebuildable cache. Tern refuses to overwrite linked development checkouts, so update the linked checkout instead.

The in-app installer refuses to run while permanent chat deletion is active, and deletion cannot start during installation. Wait for deletion to finish before running the manual command too.

---

## What it does

If you use multiple AI coding agents, your sessions are scattered across hidden SQLite databases and JSONL files. `tern-projects` unifies them into a fast, native sidebar:

- 📁 **Curated "My Projects" & Multi-Folder Workspaces** — Real projects rarely live in a single directory. Group an app, its backend API, shared libraries, and documentation into one named project with a custom icon.
- 🎯 **Primary Launch Directories** — Pick one folder as **Primary** (where fresh agent sessions spawn), while additional directories are automatically supplied via `--add-dir` to harnesses that support multi-folder workspaces (OMP, Claude Code, Codex).
- 🔍 **Instant Full-Text Search** — FTS5-powered search across all your past sessions. Filter by **Worktree**, **Repository**, or **All history** with live keyword highlighting.
- 🚀 **Fresh Chat Launching** — Press `⌘N` (`Ctrl+N`) to launch a new chat in your preferred CLI (`omp`, `claude`, `codex`, `pi`, `opencode`, or `cursor`).
- 📌 **Global Pins & Recents** — Keep critical threads pinned to the top of your sidebar. Recents keeps your 15 most recent chats one click away.
- 🗄️ **Local Archiving & Safe Deletion** — Archive threads to hide them without losing data, or permanently delete sessions via verified native CLI calls.

![tern-projects details, menus, and search results](docs/screenshots/showcase-details.png)

---

## Keyboard-First Controls

Optimized for speed and one-handed navigation:

| Action | macOS | Windows / Linux |
|---|---|---|
| **Toggle Projects sidebar** | `⌘ ⌥ S` | `Ctrl + Alt + S` |
| **Search everywhere** | `⌃ ⌥ ⌘ S` | `Ctrl + Alt + Shift + S` |
| **Cycle views** (Projects ➔ Found ➔ Search ➔ Settings) | `Tab` / `Shift + Tab` | `Tab` / `Shift + Tab` |
| **Quick Search** from Projects or Settings | `/` | `/` |
| **New chat** (in current project/folder) | `⌘ N` | `Ctrl + N` |
| **New chat with…** (choose harness) | `⇧ ⌘ N` | `Shift + Ctrl + N` |
| **Resume chat** | `Enter` | `Enter` |
| **Fold / unfold project** | `←` / `→` | `←` / `→` |
| **Context menu / Actions** | `Shift + F10` | `Shift + F10` |
| **Copy resume command** | `⌃ Y` | `Ctrl + Y` |
| **Dismiss / Clear search / Close modal** | `Esc` | `Esc` |

Inside any text field, standard terminal editing shortcuts just work (`⌘A`/`Ctrl+A` select all, `⌘⌫`/`Ctrl+U` delete line prefix, `⌥⌫`/`Ctrl+⌫` delete word).

---

## Supported Coding Harnesses

| Agent | Fresh Launch Command | Native Resume | Multi-Folder Support | Deletion Support |
|---|---|---|---|---|
| **OMP** | `omp --cwd <primary> [--add-dir <extra>...]` | `omp --resume <path>` | ✅ Multiple `--add-dir` | Transcript, artifacts, and session-attributed prompt history |
| **Claude Code** | `claude [--add-dir <extra>...]` | `claude --resume <id>` | ✅ Multiple `--add-dir` | Exact session history filtering |
| **Codex** | `codex --cd <primary> [--add-dir <extra>...]` | `codex resume <id>` | ✅ Multiple `--add-dir` | `codex delete <uuid> --force` |
| **Pi** | `pi` | `pi --session <path>` | Single root | Exact file unlinking |
| **OpenCode** | `opencode <primary>` | `opencode --session <id>` | Single root | `opencode session delete <id>` |
| **Cursor** | `cursor-agent` / native CLI | Resumes in directory | Directory | Clean database record pruning |

OMP keeps one history entry per distinct prompt and associates it with the most recent session that used it. Deleting a chat removes entries currently attributed to that session; entries attributed to another session or with no recorded session are retained.

Permanent deletion refuses unknown history schemas or triggers, ambiguous source identities, busy stores, and stale confirmations. History cleanup is checked under a write transaction before transcripts are removed; unrelated chats, unattributed prompt history, and project files are retained. Filesystem removal and SQLite commit are not crash-atomic: a process crash or later filesystem/commit failure can leave partial cleanup. Such failures are reported, and the plugin does not automatically retry.

---

## How It Works

- **Read-only indexing**: indexing and search never alter project files or original transcripts. Only explicitly confirmed permanent chat deletion removes verified harness-owned data.
- **Fast SQLite FTS**: Full-text indexing runs in a separate local SQLite database (`index.sqlite`), refreshing in the background every two minutes.
- **Durable Library**: Your custom projects, icons, pins, and preferences are stored in an independent `library.sqlite` so reindexing the cache never wipes your setup.

---

## Contributing

Contributions are welcome! Whether you are fixing a bug, improving Luau block performance, or adding support for a new AI coding assistant, see [CONTRIBUTING.md](CONTRIBUTING.md) for architecture notes and step-by-step instructions.

---

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes.

---

## License

[MIT](LICENSE) © 2026 thegeneralist01
