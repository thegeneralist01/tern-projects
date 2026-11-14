use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;


use clap::{Parser, Subcommand};
use rusqlite::Connection;

use serde_json::json;

mod model;
mod text;
mod git;
mod db;
mod adapters;
mod library;
mod sessions;
mod harnesses;
mod delete;
mod transcript;

use adapters::Adapter;
use anyhow::{Context, Result};

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "tern-projects", about = "Organize, launch and search agent chats")]
struct Cli {
    /// Path to the index SQLite database.
    #[arg(long, global = true, required = false)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan all agent stores and update the index.
    Index,

    /// Query the index for sessions.
    Query {
        #[arg(long, default_value = "")]
        q: String,
        #[arg(long, default_value = "all")]
        scope: String,
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(long, default_value = "updated")]
        sort: String,
        #[arg(long, default_value_t = 50)]
        limit: i64,
        #[arg(long, default_value_t = 0)]
        offset: i64,
        #[arg(long)]
        project_id: Option<String>,
    },

    /// List projects and recent sessions.
    Projects {
        #[arg(long, default_value_t = 25)]
        per: i64,
        #[arg(long, default_value_t = 15)]
        recent: i64,
    },

    /// Read one library operation JSON object from stdin.
    Library,
    /// Copy legacy SQLite data without modifying its source.
    MigrateData {
        #[arg(long)]
        from: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    // Determine HOME.
    let home_str = std::env::var("TERN_PROJECTS_HOME")
        .or_else(|_| std::env::var("HOME"))
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let home = PathBuf::from(&home_str);

    let db_path = match cli.db {
        Some(p) => p,
        None => {
            eprintln!("error: --db <PATH> is required");
            std::process::exit(1);
        }
    };

    match cli.command {
        Command::Index => run_index(&db_path, &home),
        Command::Query { q, scope, cwd, sort, limit, offset, project_id } => {
            emit(sessions::query(&db_path, &home_str, sessions::Query {
                text: &q, scope: &scope, cwd: &cwd, sort: &sort,
                limit, offset, project_id: project_id.as_deref(),
            }))
        }
        Command::Projects { per, recent } => {
            emit(sessions::snapshot(&db_path, &home_str, "discovered", per, recent))
        }
        Command::Library => emit(run_library(&db_path, &home)),
        Command::MigrateData { from } => emit(migrate_data(&db_path, &from)),
    }
}

// ── Index ────────────────────────────────────────────────────────────────────

fn run_index(db_path: &Path, home: &Path) {
    let _lock = match operation_lock(db_path) {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("error: {e:#}");
            println!("{}", json!({"busy": true}));
            std::process::exit(1);
        }
    };

    let conn = match db::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            println!("{}", json!({"error": e.to_string()}));
            std::process::exit(1);
        }
    };

    let t0 = Instant::now();
    let mut scanned: usize = 0;
    let mut changed: usize = 0;
    let mut removed: usize = 0;
    let mut errors: Vec<serde_json::Value> = Vec::new();
    let mut git_cache: HashMap<String, git::GitInfo> = HashMap::new();

    let mut adapters: Vec<Box<dyn Adapter>> = vec![
        Box::new(adapters::claude::ClaudeAdapter),
        Box::new(adapters::codex::CodexAdapter),
        Box::new(adapters::omp::OmpAdapter),
        Box::new(adapters::pi::PiAdapter),
        Box::new(adapters::opencode::OpenCodeAdapter),
        Box::new(adapters::cursor::CursorAdapter),
    ];

    for adapter in &mut adapters {
        let agent = adapter.agent();

        // Get candidates.
        let candidates = match adapter.candidates(home) {
            Ok(c) => c,
            Err(e) => {
                if errors.len() < 20 {
                    errors.push(json!({"source": agent, "error": e.to_string()}));
                }
                continue;
            }
        };

        let live_sources: Vec<String> = candidates.iter().map(|c| c.source.clone()).collect();
        scanned += candidates.len();

        // One transaction per adapter.
        let tx_result: anyhow::Result<(usize, usize)> = (|| {
            let mut adapter_changed = 0usize;
            let mut adapter_errors: Vec<serde_json::Value> = Vec::new();

            conn.execute_batch("BEGIN;")?;

            for cand in &candidates {
                let stored_version = db::get_source_version(&conn, &cand.source);
                let needs_parse = stored_version.as_deref() != Some(cand.version.as_str());
                if !needs_parse {
                    continue;
                }

                match adapter.parse(home, cand) {
                    Ok(Some(mut parsed)) => {
                        // Git resolution.
                        let git_info = git::resolve(&parsed.cwd, &mut git_cache);
                        let project_root = git_info.project_root.clone();
                        let worktree_root = git_info.worktree_root.clone();
                        // Stored branch (from Claude/Codex) wins over computed.
                        let branch = parsed.branch.clone().or(git_info.branch);

                        // Set resume.cwd to the session cwd now that we know it.
                        parsed.resume.cwd = parsed.cwd.clone();

                        let resume_json = serde_json::to_string(&parsed.resume)?;
                        let uid = format!("{}:{}", agent, parsed.id);

                        db::upsert_session(&conn, &db::UpsertData {
                            uid: &uid,
                            agent,
                            id: &parsed.id,
                            source: &cand.source,
                            title: &parsed.title,
                            cwd: &parsed.cwd,
                            project_root: &project_root,
                            worktree_root: &worktree_root,
                            branch: branch.as_deref(),
                            model: parsed.model.as_deref(),
                            created_ms: parsed.created_ms,
                            updated_ms: parsed.updated_ms,
                            messages: parsed.messages,
                            last_assistant: &parsed.last_assistant,
                            resume_json: &resume_json,
                            body: &parsed.body,
                            source_version: &cand.version,
                        })?;
                        adapter_changed += 1;
                    }
                    Ok(None) => {
                        // Empty session: delete any existing row, record source.
                        db::delete_by_source(&conn, &cand.source)?;
                        db::upsert_source_only(&conn, &cand.source, agent, &cand.version)?;
                        adapter_changed += 1;
                    }
                    Err(e) => {
                        if adapter_errors.len() < 20 {
                            adapter_errors.push(json!({
                                "source": &cand.source,
                                "error": e.to_string()
                            }));
                        }
                        // Leave unchanged; next run retries.
                    }
                }
            }

            conn.execute_batch("COMMIT;")?;
            errors.extend(adapter_errors);
            Ok((adapter_changed, 0))
        })();

        match tx_result {
            Ok((c, _)) => changed += c,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                if errors.len() < 20 {
                    errors.push(json!({"source": agent, "error": e.to_string()}));
                }
            }
        }

        // Remove stale rows (outside the per-source transaction).
        match db::remove_stale(&conn, agent, &live_sources) {
            Ok(n) => removed += n,
            Err(e) => {
                if errors.len() < 20 {
                    errors.push(json!({"source": format!("{agent}/stale"), "error": e.to_string()}));
                }
            }
        }
    }

    let ms = t0.elapsed().as_millis() as u64;
    println!(
        "{}",
        json!({
            "scanned": scanned,
            "changed": changed,
            "removed": removed,
            "ms": ms,
            "errors": errors
        })
    );

}

fn emit(result: Result<serde_json::Value>) {
    match result {
        Ok(value) => println!("{value}"),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

pub fn operation_lock(db_path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new().write(true).create(true)
        .truncate(false).open(db_path.with_extension("sqlite.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("index operation busy; wait for indexing or deletion to finish")?;
    Ok(file)
}

fn run_library(db_path: &Path, home: &Path) -> Result<serde_json::Value> {
    use library::Request;
    let request: Request = serde_json::from_reader(std::io::stdin().lock())
        .context("invalid library request")?;
    let home_str = home.to_string_lossy();
    match request {
        Request::Snapshot { view, per, recent } => sessions::snapshot(db_path, &home_str, &view, per, recent),
        Request::ProjectSave { id, name, icon, folders, cwd } => {
            let (index, mut lib) = sessions::open(db_path)?;
            let saved = lib.save_project(id.as_deref(), &name, &icon, &folders, &cwd, home, &index)?;
            sessions::saved_project(db_path, &home_str, saved["project"]["id"].as_str().context("missing saved project ID")?)
        }
        Request::ProjectRemove { id } => library::Library::open(db_path)?.remove_project(&id),
        Request::ChatUpdate { uid, pinned, archived, assignment } => {
            library::Library::open(db_path)?.update_chat(&uid, pinned, archived, assignment)
        }
        Request::Archived { limit, offset } => sessions::archived(db_path, &home_str, limit, offset),
        Request::SettingsUpdate { default_harness, icon_style } => {
            library::Library::open(db_path)?.update_settings(default_harness, icon_style)
        }
        Request::Folders { path, cwd, offset, limit } => library::folders(&path, &cwd, home, offset, limit),
        Request::Harnesses => harnesses::list(home),
        Request::LaunchPlan { harness, project_id, cwd } => {
            let folders = match (project_id, cwd) {
                (Some(id), None) => library::Library::open(db_path)?.project(&id)?.folders,
                (None, Some(cwd)) => vec![library::expand(&cwd, &cwd, home)?.to_string_lossy().into_owned()],
                _ => anyhow::bail!("launch-plan requires exactly one of project_id or cwd"),
            };
            harnesses::launch(db_path, home, &harness, &folders)
        }
        Request::DeletePreview { uid } => delete::preview(db_path, home, &uid),
        Request::Delete { uid, expected_source, expected_version } => {
            delete::execute(db_path, home, &uid, &expected_source, &expected_version)
        }
        Request::SourceIgnore { path, cwd } => library::Library::open(db_path)?.ignore_source(&path, &cwd, home),
        Request::SourceUnignore { path } => library::Library::open(db_path)?.unignore_source(&path),
        Request::SourceIgnores => Ok(json!({"ignored_sources": library::Library::open(db_path)?.ignored_sources()?})),
        Request::Transcript { uid } => transcript::preview(db_path, home, &uid),
    }
}

fn migrate_data(db_path: &Path, legacy: &Path) -> Result<serde_json::Value> {
    let _lock = operation_lock(db_path)?;
    let mut copies = vec![(legacy.join("index.sqlite"), db_path.to_owned())];
    let old_library = legacy.join("library.sqlite");
    if old_library.exists() {
        copies.push((old_library, library::path(db_path)));
    }
    for (source, destination) in &copies {
        anyhow::ensure!(source.is_file(), "legacy database missing: {}", source.display());
        anyhow::ensure!(!destination.exists(), "destination already exists; nothing overwritten: {}", destination.display());
    }
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for (source, destination) in &copies {
            // create_new reserves the destination against a concurrent migration.
            std::fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
            created.push(destination.clone());
            let source_conn = db::open_readonly(source)?;
            let mut target = Connection::open(destination)?;
            let backup = rusqlite::backup::Backup::new(&source_conn, &mut target)?;
            backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for destination in created {
            // Only files exclusively created by this invocation are eligible for rollback.
            let _ = std::fs::remove_file(destination);
        }
        return Err(error).context("migration failed; legacy source was not modified");
    }
    Ok(json!({"ok": true, "copied": copies.iter().map(|(_, target)|target).collect::<Vec<_>>()}))
}
