# Changelog

All notable changes to `tern-projects` will be documented in this file.

## [Unreleased]

### Fixed
- **Projects commands and New chat on Tern 0.9.5**: `Projects`, `Projects: Search` and `Projects: Settings` now open the panel, and **New chat** opens its tab. Tern 0.9.5's `cx.session:pane(id)` and `cx.layout:split(id, …)` can return nil for a live pane id, so the window now looks panes up through `cx.session:panes()` and opens the panel with `cx:new_block(…, "split")` when `split` finds no target.
- Thanks to [Florian Séran (@flosrn)](https://github.com/flosrn) for this fix, imported from [commit `abcdcdf`](https://github.com/flosrn/tern-projects/commit/abcdcdfc87f85868d38835e5d7fbaa0fb7b3648e).
- **New chat no longer goes quiet.** `New chat` with a default harness now asks only for that launch plan instead of surveying every installed CLI first (a survey that probes each tool with a 5 second cap and showed nothing meanwhile). The panel shows `Preparing new chat…` with a `Cancel` action, explains itself after 5 seconds, and gives up after 15 seconds (`launch-plan`) or 65 seconds (tool survey) with a retryable error — or names the build when `cli.status()` says one is running. Further clicks during a wait are ignored and the New chat buttons are disabled.
- **A backend that fails to start is reported.** `tern.process.run` raises when the executable cannot be spawned; that error escaped `lib/cli.luau` before any callback ran, leaving the panel waiting forever. Spawns are now wrapped and reported as `Cannot start project backend: …` through the normal error path, and each request completes at most once (timeout, non-zero exit, bad JSON, spawn failure, success).
- **A failing build waiter no longer strands the others.** `backend.ensure` reports the build result to every waiter even when one of them raises; the failure is logged instead of aborting the loop.
- **A queued launch that was cancelled never starts.** `cli.request`/`panel.request` can drop a request whose caller lost interest (`launch` and `harnesses` channels pass a liveness/mode/flow predicate), so a launch that timed out cannot spawn a process and open a late tab once a build finishes.
- **Failing launches explain themselves.** An unavailable default harness opens the picker with the reason (and selects the first usable tool); other launch failures surface a banner and toast instead of nothing; a missing primary folder still offers `Edit project…`.
- **A launch whose panel lost focus says so.** The panel records a one-shot nonce with its launch, so `window.luau` can tell its own launch (now `Chat not started` / `Focus the Projects panel and try again.`) from unrelated `tern-projects://launch` text in a terminal, and no longer toasts `Starting …` before the tab exists.

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
