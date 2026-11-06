# Contributing to tern-projects

Thank you for your interest in contributing to `tern-projects`!

`tern-projects` is a plugin for the [Tern terminal](https://stencil.so/tern). It consists of two coordinated halves:
1. **Frontend (Luau UI)**: `host.luau`, `window.luau`, `blocks/panel.luau`, and `lib/` modules driving Tern's native layout, elements, and styles.
2. **Backend (Rust CLI)**: `indexer/`, a high-performance companion CLI (`bin/tern-projects`) that manages SQLite FTS5 search indexing (`index.sqlite`), durable project metadata (`library.sqlite`), executable discovery, and safe session operations.

---

## Contributing with AI Coding Agents

Whether you are writing code by hand or pairing with an AI coding assistant (like Claude Code, Cursor, OMP, Codex, Pi, or OpenCode), contributions are welcome!

### The Role of `AGENTS.md`
If you or your AI assistant are modifying the project, **[AGENTS.md](AGENTS.md)** is the authoritative guide for this repository. It documents:
- The complete repository layout and module responsibilities.
- The host/window process architecture.
- Core invariants (such as storage isolation between `library.sqlite` and `index.sqlite`, and fail-closed deletion safety).
- Build, test, and verification commands.
- Coding conventions for Luau, Rust, and Jujutsu (`.jj`).

### When to Update `AGENTS.md`
To keep this repository easy to maintain for both human developers and future AI agent runs, please update `AGENTS.md` whenever your PR:
- **Adds, moves, or removes files**: Keep the `Repo layout` tree in `AGENTS.md` accurate.
- **Changes core invariants**: Such as modifying storage schemas, background build behavior, or safety requirements for chat deletion.
- **Updates commands or tools**: Changes to test suites, build scripts, or plugin lifecycle hooks.
- **Establishes new conventions**: New patterns across Luau UI components or Rust modules.

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

- **Rust**: locked-dependency tests and Clippy correctness/suspicious-pattern checks on Linux, macOS, and Windows.
- **Luau**: compile runtime sources at the repository root and under `blocks/` and `lib/`, and require `--!strict` on the first line. Generated `*.d.luau` declarations are excluded. This checks syntax, not Tern SDK types or native UI behavior.
- **Workflows and shell**: actionlint validates GitHub Actions, including embedded shell commands; ShellCheck checks `scripts/*.sh`.
- **PR checks**: a stable aggregate status that fails if any dependency fails, is cancelled, or is skipped. Configure this status as required in the repository's `main` branch ruleset to block merges.

Fork PRs need no secrets. The workflow uses read-only repository permissions, checkout without persisted credentials, a commit-pinned action, and checksum-verified tool downloads. GitHub may still require a maintainer to approve runs from first-time contributors.

Run the same checks locally:

```sh
cargo clippy --locked --manifest-path indexer/Cargo.toml --all-targets -- -D clippy::correctness -D clippy::suspicious
cargo test --locked --manifest-path indexer/Cargo.toml
python3 scripts/check-luau.py
actionlint
shellcheck scripts/*.sh
```

Install [Luau 0.745](https://github.com/luau-lang/luau/releases/tag/0.745) (`luau-compile`), [actionlint 1.7.12](https://github.com/rhysd/actionlint/releases/tag/v1.7.12), and [ShellCheck 0.11.0](https://github.com/koalaman/shellcheck/releases/tag/v0.11.0) on your `PATH`. The Luau checker also accepts `--compiler /path/to/luau-compile`. Keep tool versions and Linux archive checksums together in the workflow when upgrading them.

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
5. Open a Pull Request on GitHub describing your changes and testing evidence.
