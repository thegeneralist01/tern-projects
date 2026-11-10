# AGENTS.md

`tern-projects` is a Tern plugin for curated projects, AI chat history search, and chat launch/resume. The Luau frontend invokes a Rust CLI over stdin/stdout JSON; SQLite stores the disposable history index and durable user library separately.

## Contribution workflow

- Open an issue before a PR, or use an existing issue covering the change. Link it in the PR and describe changes and testing evidence. See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and submission details.
- Prefer Jujutsu (`jj`) for version-control tasks; use Git if `jj` is unavailable. Do not run destructive version-control commands unless explicitly requested.
- Keep changes focused. Update this file when the code map, commands, conventions, or core invariants change; do not turn it into a change log or an exhaustive file inventory.
- Maintainers handle version bumps when publishing releases. Do not change `plugin.toml`'s version unless the maintainer requests a release; commits without a version increase do not trigger update notifications.

## Build and verification

Prerequisites: Tern, stable Rust, and the check tools listed below. Run commands from the repository root.

```sh
./scripts/build.sh                                    # Build release binary into bin/tern-projects
cargo test --locked --manifest-path indexer/Cargo.toml # Backend unit and integration tests
cargo clippy --locked --manifest-path indexer/Cargo.toml --all-targets -- -D clippy::correctness -D clippy::suspicious
python3 scripts/check-luau.py                          # Requires luau-compile 0.745
actionlint                                           # Requires actionlint 1.7.12
shellcheck scripts/*.sh                              # Requires ShellCheck 0.11.0
tern plugin types .                                  # Regenerate tern.d.luau after Tern updates
tern plugin link .                                   # Link plugin to local Tern instance
tern plugin reload                                   # Reload plugin in running Tern instance
```

- Run checks relevant to the change. Rust tests and Clippy correctness/suspicious-pattern checks run on Linux, macOS (Intel), and Windows in CI. Formatting and existing style/dead-code warnings are not gates; avoid unrelated reformatting.
- Luau checks compile runtime files at the root and under `blocks/` and `lib/`, excluding generated declarations. They do not check SDK types or native UI behavior; verify UI changes in Tern.
- For PR inactivity automation, run `python3 -B -m unittest discover -s scripts -p 'test_pr_staleness.py'`. The scheduled workflow has label/comment write permissions; fork CI remains read-only. It tracks activity in one bot-owned PR comment so its own comments and labels do not reset the clock; never close PRs as part of stale labeling.
- Rust integration tests use real CLI executions and isolated temporary homes (`tests/fixtures/home/`). Keep fixtures cross-platform: select platform-specific storage paths, serialize paths into JSON, compare roots as paths, and generate synthetic Git metadata in the temporary home rather than relying on tracked nested `.git` files.
- CI runs on PRs, pushes to `main`, and manual dispatch. Preserve the stable `PR checks` aggregate: failure, cancellation, or skipped dependencies must fail it. Keep fork runs secret-free, permissions read-only, checkout credentials unpersisted, actions commit-pinned, and downloaded tools checksum-verified. See [CONTRIBUTING.md](CONTRIBUTING.md#pull-request-checks) for tool setup and branch protection.

## Repo layout

| Area | Responsibility |
| --- | --- |
| `host.luau` | Host process: start update checks, register the panel, manage shared state. |
| `window.luau` | Window process: shortcuts, native settings, toggles, `tern-projects://launch` and `tern-projects://resume` routes. |
| `blocks/panel.luau`, `styles.css` | Projects, Search, and Settings UI. |
| `lib/backend.luau`, `lib/cli.luau` | Cargo builds, binary cache, request queue, CLI JSON, deletion/update coordination. |
| `lib/updates.luau` | Host-only update checks, consent-gated installation, recovery. |
| `lib/input.luau`, `lib/textfield.luau` | Shortcuts, UTF-16 native edits, UTF-8 text fields. |
| `lib/ui.luau`, `lib/picker.luau`, `lib/project-editor.luau`, `lib/fmt.luau` | Shared widgets, selection sheets, project drafts/directory browser, formatting. |
| `indexer/src/main.rs`, `indexer/src/model.rs`, `indexer/src/text.rs`, `indexer/src/git.rs` | CLI entry, data types, quoting/timestamps, Git/worktree resolution. |
| `indexer/src/db.rs`, `indexer/src/library.rs`, `indexer/src/sessions.rs` | Disposable index, durable library, session projections/scopes/paging. |
| `indexer/src/harnesses.rs`, `indexer/src/adapters/`, `indexer/src/delete*.rs` | Verified harness discovery/launch, session parsing, fail-closed source deletion. |
| `indexer/tests/` | Real-binary integration tests. |
| `scripts/build.sh`, `scripts/check-luau.py`, `.github/workflows/ci.yml` | Release build, Luau compilation, CI checks. |
| `scripts/pr_staleness.py`, `scripts/test_pr_staleness.py`, `.github/workflows/pr-staleness.yml` | Daily PR inactivity labels, persistent activity clock, transition tests. |
| `plugin.toml`, `CHANGELOG.md`, `CONTRIBUTING.md`, `docs/` | Manifest, release notes, contributor workflow, technical guides and showcase assets. |
| `bin/tern-projects`, `tern.d.luau` | Gitignored build output and generated Tern declarations. |

For a new harness, follow [docs/adding-a-harness.md](docs/adding-a-harness.md) rather than inventing a second adapter pattern.

## Code conventions

- **Luau:** every runtime file starts with `--!strict`. Window scripts must not call host-only APIs (`tern.fs.*`, `tern.pane.list`). Guard nil execution contexts and pane/generation liveness before applying asynchronous CLI results. Use `lib/input.luau` for native UTF-16 edit conversion.
- **Rust:** keep adapters isolated in `indexer/src/adapters/` and use `anyhow` for consistent errors. Multi-table or multi-step database mutations require transactions and a 5000ms busy timeout. Use clean cutovers: no deprecated paths, dead feature flags, or compatibility shims.

## Storage

- `index.sqlite` is rebuildable; index schema version bumps force reindexing. Never drop or clear durable `library.sqlite` during index rebuilds: it owns curated projects, folders, pins, chat assignments, and global settings.
- Library schema v2 retains automatic memberships in `project_chats(project_id,uid)`. When saving changed source folders, capture matching indexed chats (including archived chats) under the library write lock before replacing folders. Explicit `project`/`none` assignments override retention; old paths must not remain live matching rules.
- Retained membership survives index rebuilds, cascades on project removal, and is cleared for verified UIDs during permanent chat deletion. Upgrade schema v1 transactionally without overwriting projects, annotations, or settings.

## Backend builds

- `lib/backend.luau` builds with Cargo at plugin load, caches the binary in `<plugin.data>/backend/`, and queues requests until the build settles.
- Resolve Cargo from absolute host `PATH` entries, then `CARGO_HOME/bin` (or `~/.cargo/bin`), then common Unix installation locations, with native executable lookup as the final fallback. Add the resolved Cargo directory to build `PATH` on every platform. Do not append Unix system directories on Windows or introduce empty segments when `PATH` is unset. `tern.process.run` extends the inherited environment.
- On macOS, use `/usr/bin/xcrun --sdk macosx --show-sdk-path` for `SDKROOT` and `/usr/bin/clang` for native C compilation and both supported Cargo target linkers. Other platforms keep Cargo's defaults.
- Retain complete Cargo output in Tern's log; summarize compiler errors ahead of progress/warnings in the UI. SDK discovery and asynchronous Cargo spawn failures must settle every queued caller.

## Plugin updates

- Start `lib/updates.luau` before the Rust backend. Automatic checks compare the official `main/plugin.toml` stable numeric version at most every 6 hours (`CHECK_INTERVAL`). Keep preferences, attempt/cache timestamps, and pending-install recovery in Tern KV, separate from the library/index.
- Installation requires explicit version-bound UI approval or opt-in automatic installation (off by default and dependent on automatic checks). Invoke only the resolved Tern executable with fixed `plugin install github.com/thegeneralist01/tern-projects --force` arguments, never shell commands from remote metadata. The installer fetches latest official `main`, not a pinned version; success requires disk confirmation of at least the last checked version.
- Discover the installer through absolute host `PATH` entries, user-local Unix CLI/macOS application locations, then native executable lookup. Discovery/spawn failures clear pending approval and release the installation guard. Automatic installation attempts each version at most once, never automatically retries failures, and leaves manual review available. Preserve Tern's protection of linked development checkouts.
- Pending installation status survives reload. On disk-confirmed success, persist `completed_version` and remove pending state. `window.luau` delivers the **Updated!** toast after reload without a panel, acknowledging it under `plugin-update-toast:v1`; the window must not rewrite host-owned updater preferences.
- At startup, clear a saved installation failure only when the installed version reaches its target `latest_version`. Keep check failures with no target and failures targeting newer versions.
- A shared host guard excludes installation and active permanent deletion, including queued deletions that become ready during installation. Update checks never modify the library/index.

## Deletion safety

- Fail closed: verify source identity and idle store conditions before removing files or records. Never delete project files, repositories, or worktrees.
- Use verified native CLI deletion commands for Codex/OpenCode. For OMP, Claude, Pi, and Cursor, remove only verified owned session files/artifacts; refuse if the process is running or store ownership cannot be proven.
- OMP cleanup accepts only verified table/FTS schemas and bounded indexing triggers. Delete exact binary-matched session IDs and verified owned dependents; preserve unattributed history. Refuse unknown triggers, foreign-key dependencies, or ambiguous indexed/unindexed dependent identities.
- Revalidate OMP confirmation after acquiring the history write lock with a 5000ms busy timeout. SQL deletion and FTS repair must succeed before transcript removal. Filesystem removal and SQLite commit are not crash-atomic: report partial failures without automatic retry.

## Keyboard shortcuts and chat launch

- Global shortcuts belong to Tern's `settings.json` `keybinds`. Report `cx.actions:keys()` effective bindings to Settings; never store a second keymap in the library, KV, or pane state. Keep panel-only actions/navigation as a separate fixed reference.
- `tern-projects://shortcuts?`, `keyboard?`, and `keyconfig?` require a live focused Projects panel in the calling window. They only report bindings or invoke fixed `preferences` / `open_config` commands; never accept arbitrary commands or file paths.
- While Settings is open, request a keymap snapshot every second. Stop stale polling through pane/generation liveness; identical snapshots must not redraw or disturb dialogs. Tern's Keyboard editor may omit unbound plugin actions: expose copyable IDs and the native settings-file command without inventing defaults.
- Fresh chats use the primary project directory as `cwd`. Pass additional folders through supported harness flags (such as `--add-dir`), or warn before launching primary-only. Quote commands per OS: `shell_join` on Unix, Base64-encoded command blocks on Windows.
- Fresh-chat URLs carry a per-nonce `launch:<nonce>` claim bound to the exact pane, command, and directory. Claims expire after 10 seconds and must be consumed before tab creation; they never replace a live focused Projects panel on the local host.
