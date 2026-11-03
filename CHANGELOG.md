# Changelog

All notable changes to `tern-projects` will be documented in this file.

## [0.2.5] — 2026-11-03

### Fixed
- **macOS backend builds**: use the selected Apple SDK and compiler/linker together. An older linker inherited through `PATH` can otherwise fail on newer Command Line Tools SDK architecture entries even when Rust and Cargo are installed.
- **Build diagnostics**: surface compiler errors instead of leading progress/warning output, keep complete Cargo output in Tern's log, and distinguish SDK discovery, Cargo startup, and compilation failures.

## [0.2.4] — 2026-11-03

### Added
- **Plugin update checks**: checks the official stable manifest version at startup when due and every 24 hours, with a persistent opt-out, Check now, and available-update notice.
- **Consent-based installation**: Settings shows installed/latest versions, offers Cancel-first installation review, and always provides the README's manual command. Installation uses Tern's supported installer, verifies the resulting version from disk, recovers status after plugin reload, and never overwrites a linked checkout.
- **Deletion-safe updates**: the in-app installer and permanent chat deletion exclude one another, including deletion requests queued during an installation. Checks remain independent of the Rust backend.

### Fixed
- **OMP permanent deletion**: removes prompt history attributed to the selected session and verified owned dependents, while preserving other sessions and unattributed prompts. Unsupported schemas, trigger side effects, ambiguous identities, busy stores, and stale confirmations refuse before source removal; SQL statement failures roll back before unlinking transcripts.
- **Projects commands and New chat on Tern 0.9.5**: `Projects`, `Projects: Search` and `Projects: Settings` now open the panel, and **New chat** opens its tab. Tern 0.9.5's `cx.session:pane(id)` and `cx.layout:split(id, …)` can return nil for a live pane id, so the window now looks panes up through `cx.session:panes()` and opens the panel with `cx:new_block(…, "split")` when `split` finds no target.
- Thanks to [Florian Séran (@flosrn)](https://github.com/flosrn) for this fix, imported from [commit `abcdcdf`](https://github.com/flosrn/tern-projects/commit/abcdcdfc87f85868d38835e5d7fbaa0fb7b3648e).
- **Responsive New chat**: a configured default harness skips the full tool survey. Preparation shows progress and Cancel, with 15-second launch-plan and 65-second survey deadlines. Timed-out launch plans retain verified alternatives; failed surveys offer an explicit retry without enabling unverified tools.
- **Request lifecycle**: spawn failures reach callers, failing build callbacks no longer strand other requests, and cancelled or superseded launches cannot start queued processes or apply late results.
- **Launch authorization**: short-lived, pane- and payload-bound claims isolate concurrent launches, reject changed commands and replay, and preserve the focused local Projects-panel requirement. Cancel now works while a picker is open.
- **Windows paths**: launch plans, OMP overlays, saved project folders, and folder browsing use normal drive/UNC spellings instead of `\\?\` prefixes. Unsupported device paths and non-Unicode paths are preserved rather than silently redirected.
- Thanks to [Shengfu Zhao (@Zestful-ss)](https://github.com/Zestful-ss) for the [Windows launch fix](https://github.com/Zestful-ss/tern-projects/commit/adb7395f50fdd5331e015a8fecb7b1e76fdbd85c), [folder-storage fix](https://github.com/Zestful-ss/tern-projects/commit/7470c81b6dd21dfebc200dbfd4c7e77ba5619947), [New-chat improvements](https://github.com/Zestful-ss/tern-projects/commit/cca6635cb7081fc1c43e74ec39ab60d2c07cdcba), and [OMP history cleanup](https://github.com/Zestful-ss/tern-projects/commit/f70784911f20fb04e22a92e6c3bd5467ec6a4a6d). Original authorship is preserved; upstream safety and interaction corrections are recorded separately.

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
