# Source-folder ignores

## Goal
Persist reversible folder-level visibility rules independently of transcripts, project membership, pins, and archive state. No source deletion or per-chat annotation changes.

## Contract
- Library schema v3 adds ignore rules; migration from supported v1/v2 libraries is transactional and preserves existing state. Index rebuilds do not affect rules.
- A rule covers its normalized folder and descendants by path components, never similarly prefixed siblings. Match session cwd, repository root, and worktree root so a source repository's linked worktrees do not leak through another list.
- Reuse existing path normalization and canonical/lexical fallback, preserving supplied absolute path identity as well as resolved aliases where needed. Missing historical directories remain valid. Invalid/relative paths without an absolute cwd fail without mutation.
- Overlapping rules are independent. Removing one exact rule does not remove another; a surviving ancestor can continue hiding a descendant. Settings explains this.
- Apply visibility before counts, pagination, search ranking pages, pinned/recents, Discovered grouping and project membership projections. Archived browsing also respects ignores without changing archival state.
- Curated project definitions and folder associations remain durable. Normal project cards whose folders are all ignored and which have no remaining visible chats are hidden; mixed projects retain visible chats. Unignore restores the ordinary projections from untouched underlying state.
- JSON library requests: `source-ignore {path,cwd?}`, `source-unignore {path}`, `source-ignores {}`. List and mutation responses expose `ignored_sources: [{path: string}]`; snapshot also includes this field. Unignore uses the exact persisted path returned by the list, independent of current filesystem existence.
- Discovered source-folder menus expose **Ignore source folder…**, with confirmation identifying the path, descendant scope, and Settings recovery. Settings exposes **Ignored source folders** with full paths and individual Unignore actions. Reuse existing picker/confirmation and mutation/reload machinery; keep keyboard accessibility and async liveness checks.

## Implementation
Backend owns durable rules, normalization and a shared SQL visibility gate. Frontend owns source-folder action and settings management. No new dependency expected: existing rusqlite transactions and std::path component matching cover the contract. Candidates considered: existing workspace rusqlite and std::path -> reuse; no new generic component/dependency.

## Verification
Real CLI integration fixtures cover migration, boundaries, aliases, missing paths, overlaps, explicit/retained assignments, archives/pins, existing/future chats, search count/page consistency, reindex persistence and exact restoration. Run integrated Rust tests, correctness/suspicious Clippy gates, Luau compilation, and an isolated real-Tern UI journey. One independent review before fork push and upstream PR. No installed-plugin, live database, or existing-session mutations.
