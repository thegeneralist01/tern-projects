# AGENTS.md — tern-projects

A high-level guide for AI agents working in this repository.

## Repo layout

```
plugin.toml          Tern plugin manifest
styles.css           Theme & layout styling for panel surfaces
CHANGELOG.md         Release history and notable changes
CONTRIBUTING.md      Development workflows and PR guidelines
LICENSE              Repository license
docs/                Documentation and technical guides
  adding-a-harness.md Step-by-step guide for implementing new assistant adapters
  screenshots/       Showcase images for README and documentation
host.luau            Host process half: starts update checks and registers the panel block
window.luau          Window process half: keybind migration, toggles, launch/resume routes
blocks/panel.luau    Main UI: Projects (My Projects / Discovered), Search, Settings
lib/backend.luau     Automatic Cargo build at host startup, binary cache, request queuing
lib/cli.luau         CLI invocation, JSON decoding, deletion/update coordination
lib/fmt.luau         Formatting helpers: dates, agent labels/icons, URL encoding, snippets
lib/input.luau       Keyboard shortcuts and native UTF-16 edit handling
lib/picker.luau      Modal picker component for selection sheets
lib/project-editor.luau  Unsaved project draft state and child directory browser
lib/textfield.luau   UTF-8 text field component
lib/ui.luau          Reusable UI elements, layout widgets, and list views
lib/updates.luau     Host-only version checks, consent-gated installer, persistent recovery
indexer/             Rust backend (`tern-projects` CLI binary)
  src/main.rs        CLI subcommands: index, query, projects, library, migrate-data
  src/model.rs       Data types: Candidate, Parsed, Resume, SessionJson
  src/text.rs        String utilities: clean_user, shell_join, ISO timestamps
  src/git.rs         Filesystem-based Git repository and worktree resolution
  src/db.rs          Disposable index SQLite schema, migrations, and queries
  src/library.rs     Durable user library SQLite schema, CRUD, folder membership
  src/sessions.rs    Session projections, scopes, annotations, counts, and paging
  src/harnesses.rs   Identity-verified CLI discovery and fresh-chat launch planning
  src/delete*.rs     Fail-closed per-harness source deletion logic
  src/adapters/      Session adapters for harnesses (claude, codex, omp, pi, opencode, cursor)
  tests/             Real-binary integration test suites with isolated environments
scripts/build.sh     Build helper: compiles release binary to bin/
bin/tern-projects    Compiled binary (gitignored)
tern.d.luau          Generated Tern type stubs (gitignored)
```

## Architecture Overview

`tern-projects` is a curated project manager, search engine, and launcher for AI agent histories, built as a Tern plugin.

1. **Host / Window Split**:
   - **Host** (`host.luau`): Runs in the Tern host process. Registers blocks, manages persistent host state, and triggers on-demand background builds (`lib/backend.luau`).
   - **Window** (`window.luau`): Runs in each Tern window. Handles keyboard shortcuts, action palette commands, and dispatches IPC launch/resume routes (`tern-projects://launch`, `tern-projects://resume`).

2. **Frontend UI (Luau)**:
   - `blocks/panel.luau`: Primary sidebar UI featuring three main views: Projects (My Projects & Discovered), Search (scoped FTS), and Settings.
   - `lib/`: Reusable components (`ui.luau`, `picker.luau`, `project-editor.luau`, `textfield.luau`, `input.luau`).

3. **Backend CLI (Rust)**:
   - A native Rust binary (`indexer/`) invoked via stdin/stdout JSON requests.
   - Handles background filesystem indexing, full-text search (SQLite FTS5), metadata queries, harness detection, and atomic source chat deletions.


## Maintaining AGENTS.md

This file is the living architectural map for AI agents and human contributors working on `tern-projects`. Always keep it up to date:
- **Structural updates**: Whenever files are added, moved, or deleted, update the `Repo layout` above.
- **Architectural shifts**: If database schemas, storage separation, IPC routes, or deletion contracts change, document the new invariants here.
- **Tooling & workflow**: If build commands, test harnesses, or plugin lifecycle hooks change, update the commands section.
Keeping this file accurate prevents regressions and context drift across autonomous agent sessions.
## VCS Conventions

This repository uses Jujutsu (`.jj`) as its primary version-control interface.
- Always prefer `jj` commands (`jj status`, `jj describe`, `jj diff`, `jj new`) for VCS tasks.
- Avoid destructive or mutating `git` commands unless specifically requested.

## Build & Test Commands

```sh
./scripts/build.sh                                # Build release binary into bin/tern-projects
cargo test --manifest-path indexer/Cargo.toml     # Run backend unit and integration tests
tern plugin types .                              # Regenerate tern.d.luau after Tern updates
tern plugin link .                               # Link plugin to local Tern instance
tern plugin reload                               # Reload plugin in running Tern instance
```

## Core Invariants

- **Durable Library vs. Disposable Index**:
  - `index.sqlite` is a rebuildable cache of history transcripts. Schema version bumps force a reindex.
  - `library.sqlite` stores durable user configurations (curated projects, folder associations, custom pins, chat assignments, global settings). Never drop or clear `library.sqlite` during index rebuilds.
- **Automatic Compilation**:
  - `lib/backend.luau` automatically compiles the Rust binary with Cargo when the plugin loads and caches the binary inside `<plugin.data>/backend/`. Requests queue until compilation finishes.
  - On macOS, resolve the active Apple SDK with `/usr/bin/xcrun --sdk macosx --show-sdk-path`, set `SDKROOT`, and use `/usr/bin/clang` for native C compilation and both supported Cargo target linkers. This avoids mixing PATH-provided linkers with an incompatible SDK; other platforms retain Cargo's defaults.
  - Build failures retain complete Cargo output in Tern's log and summarize compiler errors before progress/warning noise in the UI. SDK discovery and asynchronous Cargo spawn failures must settle all queued callers.
- **Plugin Updates**:
  - `lib/updates.luau` starts before the Rust backend. Automatic checks compare the official `main/plugin.toml` stable numeric version at most every 6 hours (`CHECK_INTERVAL`); Tern KV stores updater preferences, attempt/cache timestamps, and pending-install recovery independently of `library.sqlite`.
  - Every in-app installation requires either explicit, version-bound UI approval or opt-in automatic installation (disabled by default; requires automatic checks). Installation invokes the resolved Tern executable with fixed `plugin install github.com/thegeneralist01/tern-projects --force` arguments, never a shell command from remote metadata. The installer fetches latest official `main`, not a pinned version; success requires disk confirmation of at least the last checked version.
  - Installer discovery prefers absolute host PATH entries, then user-local Unix CLI and macOS application locations, retaining native executable lookup for other layouts. Discovery and spawn failures clear pending approval and release the installation guard; failures are never retried automatically for the same version.
  - Pending installation status survives plugin reload. Automatic installation makes at most one attempt per version and never retries failures automatically, while manual review remains available. Linked development checkouts remain protected by Tern's installer.
  - At startup, a saved installation failure is cleared once the installed version reaches its target `latest_version`; check failures (no target version) and still-newer targets keep their error.
  - A shared host guard excludes in-app installation and active permanent deletion, including queued deletions becoming ready during installation. Updater checks never modify the library/index.
  - Increase `plugin.toml`'s version when publishing an update; commit changes without a version increase are not update notifications.
- **Fail-Closed Deletion Safety**:
  - Permanent chat deletion verifies source identity and idle store conditions before removing any files or database records.
  - Harnesses with native CLI commands (e.g. Codex, OpenCode) use their verified native CLI deletion subcommands.
  - Harnesses without native deletion (OMP, Claude, Pi, Cursor) remove only verified session files/artifacts and refuse if the process is running or store ownership cannot be proven.
  - OMP history cleanup accepts only verified table/FTS schemas and bounded indexing triggers, deletes exact binary-matched session IDs and verified owned dependents, and preserves unattributed history. Unknown triggers, foreign-key dependencies, or ambiguous indexed/unindexed dependent identities refuse deletion.
  - OMP confirmation is revalidated after acquiring the history write lock with a 5000ms busy timeout; SQL deletion and FTS repair must succeed before transcript removal. Filesystem removals and SQLite commit are not crash-atomic; partial failures are reported without automatic retry.
  - Never delete user project files, repositories, or worktrees during chat deletion.
- **Launch and Resume Contracts**:
  - Fresh chats launch with the primary project directory as `cwd`. Any additional folders are passed via harness-supported CLI flags (e.g. `--add-dir`), or a warning is surfaced before launching primary-only.
  - Command generation must be safely quoted per OS (`shell_join` on Unix, Base64-encoded command blocks on Windows).
  - Fresh-chat launch URLs carry a per-nonce `launch:<nonce>` claim bound to the exact pane, command, and directory. Claims expire after 10 seconds and must be consumed before tab creation; a claim never substitutes for a live focused Projects panel on the local host.

## Code Conventions

### Luau
- Every file MUST begin with `--!strict`.
- Window scripts (`window.luau`) must not call host-only APIs (`tern.fs.*`, `tern.pane.list`).
- Guard against nil execution contexts and check pane/generation liveness before applying async CLI callback results.
- Avoid text field cursor mismatches: use `lib/input.luau` for consistent native UTF-16 edit conversions.

### Rust
- Clean cutovers: avoid deprecated paths, dead feature flags, and backward-compatibility shims.
- All database operations across tables or multi-step mutations must use transactions with an active busy timeout (5000ms).
- Keep adapters isolated in `indexer/src/adapters/` with uniform error handling via `anyhow`.
- Tests must run against isolated temporary home directories (`tests/fixtures/home/`) using real CLI executions.

## Adding a New Harness / Adapter

See the complete technical guide in **[docs/adding-a-harness.md](docs/adding-a-harness.md)** for detailed implementation patterns, error handling, and test requirements.
