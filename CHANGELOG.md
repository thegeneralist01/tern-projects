# Changelog

All notable changes to `tern-projects` will be documented in this file.

## [Unreleased]

### Fixed
- **Projects commands and New chat on Tern 0.9.5**: `Projects`, `Projects: Search` and `Projects: Settings` now open the panel, and **New chat** opens its tab. Tern 0.9.5's `cx.session:pane(id)` and `cx.layout:split(id, …)` can return nil for a live pane id, so the window now looks panes up through `cx.session:panes()` and opens the panel with `cx:new_block(…, "split")` when `split` finds no target.
- Thanks to [Florian Séran (@flosrn)](https://github.com/flosrn) for this fix, imported from [commit `abcdcdf`](https://github.com/flosrn/tern-projects/commit/abcdcdfc87f85868d38835e5d7fbaa0fb7b3648e).

## [0.2.5] — 2026-10-30

### Fixed
- **Project folder storage**: saved project folders and the folder browser now record the plain spelling instead of the `\\?\` verbatim path. Existing rows keep working (launch normalizes them) and are rewritten the next time the project is saved.

## [0.2.4] — 2026-10-30

### Fixed
- **Windows launch paths**: `launch-plan` no longer hands out the verbatim `\\?\` spelling that `std::fs::canonicalize` produces. It is now stripped from the primary folder (`--cwd` / `--cd` / positional), the extra `--add-dir` folders, and the OMP `--config` overlay. Harnesses that key per-project state on the cwd crashed instead of starting: OMP builds its session directory from the cwd, keeps the `?`, and fails with `ENOENT: no such file or directory, mkdir ...\---?-D--...--` (the shell then exits 1).
- UNC folders keep their normal spelling (`\\?\UNC\server\share` → `\\server\share`).

## [0.2.3] — 2026-10-16

### Added
- **Visual hierarchy**: title-case section headings (`Pinned`, `Projects`, `Folders`, `Recents`), aligned margins, and an inline `+` button on the Projects header for immediate project creation.
- **Multi-folder workspaces**: projects group multiple source folders with a designated **Primary** launch directory; automatic `--add-dir` flag passing for supported harnesses (OMP, Claude Code, Codex).
- **One-handed keyboard navigation**: 4-stop `Tab` view cycling (`My projects` ➔ `Discovered` ➔ `Search` ➔ `Settings`), `Shift+F10` options menu, keyboard-navigable archived pages (`Ctrl+P` / `Ctrl+N`), and `Esc` hide for in-flight mutations.
- **Inspectable dock**: dedicated branch indicators (`⎇ main`) that don't truncate when directory paths are long.
- **Safe operation lifecycle**: operation errors stay inside their active sheet; in-flight tasks can be dismissed (`Esc`) without cancelling background execution.

### Changed
- Overlay cards, dialogs, and menus now use solid backgrounds (`var(--card)`) to eliminate background bleed-through.
- Unified primary buttons to match native Tern sheet styling in both light and dark appearances.

### Fixed
- Serialized SQLite first-open initialization lock to eliminate database busy errors during concurrent first launches.
- Preserved nearby row selection when an active project, pin, or chat is archived, unpinned, or deleted.

---

## [0.2.2] — 2026-10-15

### Added
- **Automatic backend build**: compiles the Rust indexer in the background on first plugin load without manual steps.
- **Product rename**: clean migration from legacy `tern-sessions` namespace to `tern-projects`.

### Fixed
- Serialized index schema migration in a single SQLite transaction with lock re-checks to prevent startup races.

---

## [0.2.0] — 2026-10-15

### Added
- Curated **My Projects** view alongside automatic **Discovered** folder groups.
- Durable project metadata storage in `library.sqlite`, isolated from the disposable search cache.
- Fresh chat launching directly into installed agent CLIs (`omp`, `claude`, `codex`, `pi`, `opencode`, `cursor`).
- Global pinned chats and 15-chat recents list.
- Local chat archiving and verified native deletion across all 6 supported harnesses.
