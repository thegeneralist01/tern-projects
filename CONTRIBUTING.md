# Contributing to tern-projects

Thank you for your interest in contributing to `tern-projects`!

`tern-projects` is a plugin for the [Tern terminal](https://stencil.so/tern). It consists of two coordinated halves:
1. **Frontend (Luau UI)**: `host.luau`, `window.luau`, `blocks/panel.luau`, and `lib/` modules driving Tern's native layout, elements, and styles.
2. **Backend (Rust CLI)**: `indexer/`, a high-performance companion CLI (`bin/tern-projects`) that manages SQLite FTS5 search indexing (`index.sqlite`), durable project metadata (`library.sqlite`), executable discovery, and safe session operations.

---

## Issues for AI-Assisted Bug Fixes

AI-assisted bug-fix PRs (not enhancement PRs!) must reference an issue. Reuse an existing issue or open one before the PR to discuss scope.

Search the [existing issues](https://github.com/thegeneralist01/tern-projects/issues) before creating a new one. Link the issue in your PR description; use `Closes #123` when the PR fully resolves it, or `Refs #123` for related work.

---

## Contributing with AI Coding Agents

Whether you are writing code by hand or pairing with an AI coding assistant (like Claude Code, Cursor, OMP, Codex, Pi, or OpenCode), contributions are welcome!

### The Role of `AGENTS.md`
If you or your AI assistant are modifying the project, follow **[AGENTS.md](AGENTS.md)** for repository-specific instructions:
- Contribution workflow and version-control conventions.
- Build commands, verification requirements, and a compact code map.
- Coding conventions and core invariants, including storage isolation and fail-closed deletion.

### When to Update `AGENTS.md`
Update `AGENTS.md` when your PR changes:
- **Code organization**: Keep the `Repo layout` map accurate for affected areas; it is not an exhaustive file inventory.
- **Core invariants**: Storage schemas, background builds, IPC routes, updates, or chat deletion.
- **Commands or tools**: Test suites, build scripts, tool versions, or plugin lifecycle hooks.
- **Coding conventions**: Rules agents need to follow when modifying Luau or Rust.

Keep instructions actionable. Put release history in `CHANGELOG.md` and detailed implementation guides in `docs/` rather than expanding `AGENTS.md` with narrative or duplicated documentation.

---

## Local Development Workflow

### Prerequisites
- [Tern](https://stencil.so/tern) (desktop release)
- Rust toolchain (`rustc`, `cargo` — current stable)

### Setup & Build

```sh
# Clone the repository
git clone https://github.com/thegeneralist01/tern-projects.git
cd tern-projects

# Build the release Rust binary and copy to bin/tern-projects
./scripts/build.sh

# Link the plugin into Tern
tern plugin link .

# Verify the plugin is recognized
tern plugin list --json
```

Whenever you modify Rust code in `indexer/`, run `./scripts/build.sh` (or `cargo test --manifest-path indexer/Cargo.toml`) to compile and test. Tern automatically reloads Luau and CSS changes when saved.

### Pull request checks

[CI](.github/workflows/ci.yml) runs on every pull request, pushes to `main`, and manual dispatch:

- **Rust**: locked-dependency tests and Clippy correctness/suspicious-pattern checks on Linux, macOS (Intel), and Windows. The explicit `macos-15-intel` runner avoids arm64 runner-allocation failures.
- **Luau**: compile runtime sources at the repository root and under `blocks/` and `lib/`, and require `--!strict` on the first line. Generated `*.d.luau` declarations are excluded. This checks syntax, not Tern SDK types or native UI behavior.
- **Chat launches**: focused Luau regressions cover fallback authorization after a CLI timeout, expiry, and replay protection.
- **Workflows and shell**: actionlint validates GitHub Actions, including embedded shell commands; ShellCheck checks `scripts/*.sh`.
- **PR inactivity automation**: unit tests cover week boundaries, progression, and activity resets.
- **PR checks**: a stable aggregate status that fails if any dependency fails, is cancelled, or is skipped. Configure this status as required in the repository's `main` branch ruleset to block merges.

Fork PRs need no secrets. The workflow uses read-only repository permissions, checkout without persisted credentials, a commit-pinned action, and checksum-verified tool downloads. GitHub may still require a maintainer to approve runs from first-time contributors.

Run the same checks locally:

```sh
cargo clippy --locked --manifest-path indexer/Cargo.toml --all-targets -- -D clippy::correctness -D clippy::suspicious
cargo test --locked --manifest-path indexer/Cargo.toml
python3 scripts/check-luau.py
python3 scripts/test_chat_launch.py
actionlint
python3 -B -m unittest discover -s scripts -p 'test_pr_staleness.py'
shellcheck scripts/*.sh
```

Install [Luau 0.745](https://github.com/luau-lang/luau/releases/tag/0.745) (`luau-compile` and `luau`), [actionlint 1.7.12](https://github.com/rhysd/actionlint/releases/tag/v1.7.12), and [ShellCheck 0.11.0](https://github.com/koalaman/shellcheck/releases/tag/v0.11.0) on your `PATH`. The Luau checker accepts `--compiler /path/to/luau-compile`; the launch regressions accept `--runtime /path/to/luau`. Keep tool versions and Linux archive checksums together in the workflow when upgrading them.

Rust formatting and existing style/dead-code warnings are not gates in this workflow; the current tree does not have a clean strict-formatting/`-D warnings` baseline. Keep changes focused rather than reformatting unrelated code.

Integration fixtures must work from a clean checkout on every matrix platform: select platform-specific storage paths, serialize filesystem paths into JSON rather than interpolating them, compare filesystem roots as paths rather than separator-sensitive strings, and generate synthetic Git metadata inside the temporary home instead of relying on nested `.git` files being tracked.

---

## Adding Support for a New AI Assistant

Interested in adding support for another AI coding assistant (e.g. Goose, Aider, Devin)?

We have a dedicated, step-by-step technical guide covering adapter discovery, launch planning, safe deletion, and UI icons:

👉 **[docs/adding-a-harness.md](docs/adding-a-harness.md)**

---

## Code Style & Conventions

- **Luau**: `--!strict` at the top of every `.luau` file. Follow existing patterns and avoid global state where possible.
- **Rust**: Standard Rust 2021 edition formatting (`cargo fmt`). Prefer direct error context with `anyhow::Context`.
- **CSS**: Scoped to `[data-surface="plugin.tern-projects.panel"]` using Tern design tokens (`var(--sans)`, `var(--card)`, `var(--t1)`, `var(--accent)`). Never hardcode light/dark colors when a theme token exists.
- **Safety**: Never run destructive git operations or mutate user repositories. Chat deletions must fail closed if process ownership or store identity is ambiguous.

---

## Submitting a Pull Request

1. Fork the repository and create your branch from `main`.
2. Run the pull request checks above.
3. Commit with a clear, conventional commit message (e.g. `feat: add goose agent adapter`).
4. If you made structural or architectural changes, verify `AGENTS.md` is updated.
5. Open a Pull Request on GitHub describing your changes and testing evidence. For AI-assisted bug fixes, link the issue opened beforehand.

## PR Inactivity

Open PRs, including drafts, are checked daily:

| Time without activity | Label |
| --- | --- |
| At least 7 days | `soft-stale` |
| At least 14 days | `medium-stale` |
| At least 21 days | `hard-stale` |

Only the current stage is kept; unrelated labels are preserved. New commits, comments, reviews, edits, and other PR activity reset the clock and clear stale labels on the next daily check. PRs are **not closed automatically**.

The bot creates one tracking comment when a PR first becomes stale and reuses it afterward. Its own tracking-comment and stale-label changes do not count as activity. The workflow also supports a manual run from GitHub Actions and starts working once it is on the default branch.
