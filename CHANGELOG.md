# Changelog

All notable changes to `tern-projects` will be documented in this file.

## [Unreleased]

### Added
- **Sidebar position memory**: closing Projects with its shortcut remembers which side of its neighbor it sat on (left, right, above or below), and the next panel opens on that side instead of always on the left. Only a panel that lands beside its neighbor is narrowed to the sidebar width. The Projects shortcut on a focused panel also closes it right after a plugin reload, instead of first only refocusing it; Search and Settings still switch it.

## [0.2.14] — 2026-11-13

### Fixed
- **Remote chat launches**: start and resume chats through the host CLI in the originating panel's window and session, preserving ACP preferences. Link fallback receives a fresh authorization lifetime after CLI failure.

## [0.2.13] — 2026-11-12

### Added
- **Source-folder ignores**: Discovered folders offer a reversible **Ignore source folder…** action; Settings lists ignored paths with **Unignore**. Rules cover existing and future descendant/worktree chats consistently across projects, recents, pins, search, and archived browsing, before counts and pagination. Library schema v3 preserves rules across index rebuilds without changing transcripts, assignments, retained memberships, pins, or archive state.
- **Pull request checks**: GitHub Actions runs backend tests and Clippy correctness/suspicious-pattern checks on Linux, macOS (Intel), and Windows, compiles runtime Luau and enforces strict headers, and validates workflows and shell scripts. A stable **PR checks** status aggregates the results; fork runs require no secrets and use read-only permissions and pinned, verified tools.

### Changed
- **Library compatibility**: source-folder ignores upgrade library schema v1/v2 to v3. Older builds cannot open v3; they refuse it without overwriting data. Back up the library before upgrading if rollback is required.
- **Portable integration fixtures**: select Cursor storage by platform, JSON-escape Windows worktree paths, compare folder and repository roots without platform-specific spelling assumptions (including concurrent first-open checks), and generate synthetic Git metadata inside the temporary home for clean CI checkouts.

### Fixed
- **ACP resumes**: Codex and Claude Code Resume open Tern’s ACP interface when automatic ACP is enabled, while respecting each agent’s own-interface preference. Copied resume commands remain portable CLI commands.

## [0.2.12] — 2026-11-06

### Fixed
- **Rust toolchain discovery and builds**: honor Cargo installations selected through the host's `PATH` before checking Cargo home and common Unix installation locations, including Homebrew. Add the resolved Cargo directory to the build `PATH` across platforms so Cargo can find `rustc` and sibling tools, while preserving Apple's SDK/compiler overrides on macOS.
- Thanks to [@lumiarna](https://github.com/lumiarna) for their first contribution in [PR #1](https://github.com/thegeneralist01/tern-projects/pull/1).

## [0.2.11] — 2026-11-05

### Added
- **Native shortcut customization**: Projects Settings shows Tern’s actual global bindings, including user overrides, unbound commands, multiple shortcuts, and key sequences. **Edit in Tern** opens native Settings; copyable action IDs and **Open settings.json** cover unbound commands absent from Tern’s Keyboard page. Panel-only controls remain a separately labeled reference, with no second shortcut configuration.

## [0.2.10] — 2026-11-04

### Added
- **Update confirmation toast**: successful manual and automatic in-app updates show **Updated!** with the installed version, even if the Projects panel closes during reload. Failed or unconfirmed installations stay silent; acknowledged successes are not repeated on subsequent reloads.

### Fixed
- **Chats survive project folder changes**: existing indexed chats remain associated with their project when its source folders are changed or renamed, including archived chats and overlapping projects. Retained associations survive reindexing and apply to project-scoped search; explicit assignment choices still take precedence. Reusing the old path does not pull unrelated new chats into the project. Historical missing paths resolve through surviving parents, including macOS filesystem aliases. Original transcripts are unchanged.

## [0.2.9] — 2026-11-03

### Changed
- **Update checks every 6 hours**: automatic checks now run at startup when the last check is at least 6 hours old, then every 6 hours, instead of daily. With Automatic installation enabled, new releases arrive within about 6 hours.

## [0.2.8] — 2026-11-03

### Added
- **Automatic installation**: opt-in automatic updates under Settings → Plugin updates (off by default). When enabled alongside automatic checks, newly checked official versions install without manual review. Automatic installation attempts at most once per version without automatic retry on failure, defers while permanent chat deletion is active, and preserves manual review after a failed attempt.

## [0.2.7] — 2026-11-03

### Fixed
- **Stale update errors**: after a terminal update installs the version a failed in-app installation targeted (or newer), Settings shows the plugin as current instead of keeping the old installer error. Check failures and failures for versions still newer than the installation remain visible.

## [0.2.6] — 2026-11-03

### Fixed
- **In-app installer discovery**: resolve the Tern executable from the host's PATH or standard user-local CLI and macOS application locations before starting the approved installation. Updates no longer depend on the GUI host finding a bare `tern` command. Installation consent, deletion exclusion, disk verification, and the single-attempt policy are unchanged.
- Older installations that cannot launch the installer can bootstrap this fix with the README's manual update command from a terminal.

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
