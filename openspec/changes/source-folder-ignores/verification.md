# Verification

## Executed locally (macOS arm64)

- Native MBX admission: repository root has no Cargo manifest; the required root attempt refused without building. `mbx-check "$PWD/indexer"` admitted the actual Cargo workspace. All Rust builds used the normal managed Cargo route; no compiler outputs were copied.
- `cargo test --locked --manifest-path indexer/Cargo.toml --test library`: 10 passed.
- Final `cargo test --locked --manifest-path indexer/Cargo.toml`: 65 passed across 5 suites.
- `cargo clippy --locked --manifest-path indexer/Cargo.toml --all-targets -- -D clippy::correctness -D clippy::suspicious`: passed; 0 errors, 12 non-gating style/dead-code warnings.
- `python3 scripts/check-luau.py`: all 12 runtime Luau files compiled and strict headers checked.
- `git diff --check`: passed.
- Concurrent-indexing regression negative control: `cargo test --locked --manifest-path indexer/Cargo.toml --bin tern-projects indexing_cannot_leak_ignored_chats_into_an_open_projection` failed with the intended visibility assertion when the read transaction was removed. Restoring `BEGIN DEFERRED` passed in the final full suite.

## Synthetic end-to-end proof

The real CLI integration fixtures cover supported-library migration, normalized paths and sibling boundaries, absent directories, independent overlapping rules, linked worktrees, preserved explicit/retained memberships, pins and archives, count/pagination consistency, future indexing, index rebuild, exact-path unignore, and original transcript bytes. A removed-symlink fixture checks retained alias-target matching. Folder edits while ignored retain automatic membership for restoration. Windows drive-relative rejection is included but not executed on this macOS host.

An isolated real `tern serve` process loaded a source-only fixture containing the production panel and CLI modules. Its private backend adapter called the checkout's MBX-managed executable directly; auto-build and updater startup were omitted. Private synthetic HOME, config, plugin data, daemon/control sockets and logs separated it from installed Tern/Projects. No binary copies, installed-plugin reloads, or owner databases were used.

Observed via native tree queries and light/dark screenshots:
- Discovered → Ignore source folder confirmation names the exact folder and scope; Cancel leaves no rule.
- Keyboard confirmation removes the source and chats from Discovered/Recents; search total changes from 2 to 1.
- Settings lists the exact persisted rule; keyboard Unignore empties the manager and restores both native search results and the source.
- Removing a child rule leaves its parent effective (0 matching chats); removing the parent restores both chats.
- A rule removed by another CLI request produces an error toast and refreshes the stale manager automatically.
- Creating a project whose only folder is ignored succeeds with explicit hidden-project/Settings recovery feedback. Native Unignore restores the project card.
- Backend-unavailable management shows an error and Retry control; empty-state management is visible.

Only the private harness was stopped after verification. Screenshots and fixture databases are local proof artifacts, not public repository content.

## Independent review

A read-only advisor review found hidden-project save feedback missing, stale unignore-list handling, a concurrent-indexing visibility race, and Windows drive-relative validation. All were repaired; the focused delta review found no blockers and approved publication after the integrated gates. Forward-only migration and alias-retarget behavior are documented in README and CHANGELOG.

## Limits

- Local execution is macOS arm64; Linux/Windows behavior awaits the existing upstream CI matrix. Luau compilation is not SDK type checking.
- Schema v3 is forward-only. Older plugin builds refuse it; retain a library backup if rollback is required.
- A vanished alias never observed/resolved cannot be reconstructed. Retargeting an ignored symlink keeps its saved old target ignored until the rule is removed.
- With rules present, each query scans indexed source paths and caches normalization per distinct path. No large-history latency claim is made.
- The automatic installer/build path was deliberately not exercised. The installed plugin and live library remain unchanged; this feature is not active there.
