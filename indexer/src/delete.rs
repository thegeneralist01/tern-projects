//! Permanent deletion is source-first: a cache row is never a substitute for deleting a chat.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

#[path = "delete_cursor.rs"]
mod cursor;
#[path = "delete_files.rs"]
mod files;
#[path = "delete_native.rs"]
mod native;

#[derive(Debug)]
struct Chat {
    uid: String,
    agent: String,
    id: String,
    source: String,
    title: String,
    version: String,
}

enum Operation {
    Files(files::Plan),
    Cursor(cursor::Plan),
    Native(native::Plan),
}
struct Plan {
    operation: Operation,
    targets: Vec<String>,
    ids: Vec<String>,
    stamps: BTreeSet<PathBuf>,
}

/// A read-only preflight, including exact owned paths/keys and source-dependent chat IDs.
pub fn preview(db_path: &Path, home: &Path, uid: &str) -> Result<Value> {
    let chat = lookup(db_path, uid)?;
    let inspected = prepare(home, &chat).and_then(|plan| {
        validate_cached_dependents(db_path, &chat, &plan)?;
        let version = token(&chat, &plan)?;
        let blocked = if matches!(plan.operation, Operation::Native(_)) {
            None
        } else {
            require_idle(home, &chat.agent)
                .err()
                .map(|e| format!("{e:#}"))
        };
        Ok((plan, version, blocked))
    });
    let (targets, ids, version, blocked) = match inspected {
        Ok((p, v, b)) => (
            p.targets,
            p.ids
                .into_iter()
                .filter(|id| id != &chat.id)
                .collect::<Vec<_>>(),
            v,
            b,
        ),
        Err(e) => (vec![], vec![], chat.version.clone(), Some(format!("{e:#}"))),
    };
    Ok(
        json!({"uid":chat.uid,"agent":chat.agent,"title":chat.title,"source":chat.source,
        "version":version,"targets":targets,"dependent_ids":ids,"blocked_reason":blocked}),
    )
}

/// Execute only an unchanged, freshly revalidated preview under the shared index lock.
pub fn execute(
    db_path: &Path,
    home: &Path,
    uid: &str,
    expected_source: &str,
    expected_version: &str,
) -> Result<Value> {
    let _lock = crate::operation_lock(db_path)?;
    let chat = lookup(db_path, uid)?;
    ensure!(
        chat.source == expected_source,
        "Chat source changed; obtain a fresh deletion preview"
    );
    let plan = prepare(home, &chat)?;
    validate_cached_dependents(db_path, &chat, &plan)?;
    ensure!(
        token(&chat, &plan)? == expected_version,
        "Chat or owned source data changed; obtain a fresh deletion preview and confirm again"
    );
    if !matches!(plan.operation, Operation::Native(_)) {
        require_idle(home, &chat.agent)?;
    }
    // A malformed durable library must fail before any irreversible source operation.
    let _library = crate::library::Library::open(db_path)?;
    // Enumerating and validating the complete operation precedes the first source write.
    match &plan.operation {
        Operation::Files(p) => files::execute(p, || {
            ensure!(
                token(&chat, &plan)? == expected_version,
                "Chat or owned source data changed while acquiring the history lock; obtain a fresh deletion preview"
            );
            Ok(())
        })?,
        Operation::Cursor(p) => cursor::execute(p)?,
        Operation::Native(p) => native::execute(home, p)?,
    }
    verify_absent(home, &chat, &plan)
        .context("Source cleanup did not complete; cache and annotations were retained")?;
    let deleted = cleanup_cache(db_path, &chat, &plan)
        .context("Original source chat was removed, but cache/annotation cleanup failed; refresh the index before another operation")?;
    Ok(json!({"ok":true,"deleted_uids":deleted}))
}

fn lookup(db_path: &Path, uid: &str) -> Result<Chat> {
    let conn = crate::db::open_readonly(db_path)?;
    conn.query_row("SELECT s.uid,s.agent,s.id,s.source,s.title,v.version FROM sessions s JOIN sources v ON v.source=s.source WHERE s.uid=?1", [uid], |r| {
        Ok(Chat{uid:r.get(0)?,agent:r.get(1)?,id:r.get(2)?,source:r.get(3)?,title:r.get(4)?,version:r.get(5)?})
    }).optional()?.context("Chat is no longer indexed; refresh before deleting")
}

fn adapter(agent: &str) -> Result<Box<dyn crate::adapters::Adapter>> {
    use crate::adapters::*;
    Ok(match agent {
        "omp" => Box::new(omp::OmpAdapter),
        "pi" => Box::new(pi::PiAdapter),
        "claude" => Box::new(claude::ClaudeAdapter),
        "codex" => Box::new(codex::CodexAdapter),
        "opencode" => Box::new(opencode::OpenCodeAdapter),
        "cursor" => Box::new(cursor::CursorAdapter),
        _ => bail!("Permanent deletion is not supported for {agent}"),
    })
}

fn prepare(home: &Path, chat: &Chat) -> Result<Plan> {
    let mut source_adapter = adapter(&chat.agent)?;
    let candidate = source_adapter
        .candidates(home)?
        .into_iter()
        .find(|c| c.source == chat.source)
        .context("Original source is missing or no longer listable; refresh the index")?;
    ensure!(
        candidate.version == chat.version,
        "Original source changed since indexing; refresh before previewing deletion"
    );
    let parsed = source_adapter
        .parse(home, &candidate)?
        .context("Original source can no longer be parsed; refusing deletion")?;
    ensure!(
        parsed.id == chat.id,
        "Original source identity does not match the indexed chat"
    );
    match chat.agent.as_str() {
        "omp" | "pi" | "claude" => files::prepare(home, chat),
        "cursor" => cursor::prepare(home, chat),
        "codex" | "opencode" => native::prepare(home, chat),
        _ => bail!("Unsupported harness"),
    }
}

fn validate_cached_dependents(db_path: &Path, chat: &Chat, plan: &Plan) -> Result<()> {
    if let Operation::Files(files) = &plan.operation {
        let conn = crate::db::open_readonly(db_path)?;
        for id in &plan.ids {
            let mut statement =
                conn.prepare("SELECT source FROM sessions WHERE agent=?1 AND id=?2")?;
            let sources = statement
                .query_map(params![chat.agent, id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for source in sources {
                ensure!(files::owns_source(files,&chat.agent,&source),"Ambiguous child-session identity: {id} is also indexed outside the owned deletion targets; refusing deletion");
            }
        }
    }
    Ok(())
}

fn token(chat: &Chat, plan: &Plan) -> Result<String> {
    let mut history_paths = BTreeSet::new();
    if let Operation::Files(p) = &plan.operation {
        files::history_stamps(p, &mut history_paths)?;
    }
    let mut stamps = Vec::with_capacity(plan.stamps.len() + history_paths.len());
    for p in plan.stamps.union(&history_paths) {
        let m =
            std::fs::symlink_metadata(p).with_context(|| format!("Inspecting {}", p.display()))?;
        ensure!(
            !m.file_type().is_symlink(),
            "Refusing symlink {}",
            p.display()
        );
        let modified = m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        stamps.push(json!([p, m.len(), modified]));
    }
    // Native stores use exact target-row contents, not whole-database/WAL timestamps.
    let native_state = match &plan.operation {
        Operation::Native(p) => Some(&p.state),
        _ => None,
    };
    #[derive(serde::Serialize)]
    struct TokenState<'a> {
        index: &'a str,
        ids: &'a [String],
        targets: &'a [String],
        stamps: Vec<Value>,
        native_state: Option<&'a Vec<Value>>,
    }
    // Confirmation metadata must never contain transcript/message bodies.
    // Stream into a digest instead of allocating another serialized transcript.
    struct DigestWriter(Sha256);
    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(
        &mut writer,
        &TokenState {
            index: &chat.version,
            ids: &plan.ids,
            targets: &plan.targets,
            stamps,
            native_state,
        },
    )?;
    Ok(format!("{:x}", writer.0.finalize()))
}

fn verify_absent(home: &Path, chat: &Chat, plan: &Plan) -> Result<()> {
    match &plan.operation {
        Operation::Files(p) => files::verify(p)?,
        Operation::Cursor(p) => cursor::verify(p)?,
        Operation::Native(p) => native::verify(home, p)?,
    }
    let candidates = adapter(&chat.agent)?.candidates(home)?;
    ensure!(
        !candidates.iter().any(|c| c.source == chat.source),
        "Original source still appears in the harness adapter"
    );
    Ok(())
}

fn cleanup_cache(db_path: &Path, chat: &Chat, plan: &Plan) -> Result<Vec<String>> {
    let ids = &plan.ids;
    // Validate the durable schema before attaching; no migrations or fallback recreation here.
    let _library = crate::library::Library::open(db_path)?;
    let mut conn =
        Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS library",
        params![crate::library::path(db_path).to_string_lossy()],
    )?;
    let tx = conn.transaction()?;
    let mut uids = vec![];
    for id in ids {
        let mut stmt = tx.prepare("SELECT uid,source FROM sessions WHERE agent=?1 AND id=?2")?;
        let rows = stmt
            .query_map(params![chat.agent, id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (uid, source) in rows {
            crate::db::delete_by_source(&tx, &source)?;
            tx.execute("DELETE FROM sources WHERE source=?1", [&source])?;
            tx.execute("DELETE FROM library.chat_state WHERE uid=?1", [&uid])?;
            uids.push(uid);
        }
        let source = match chat.agent.as_str() {
            "cursor" => Some(format!("composerData:{id}")),
            "codex" | "opencode" => Some(format!("{}:{id}", chat.agent)),
            _ => None,
        };
        if let Some(source) = source {
            tx.execute("DELETE FROM sources WHERE source=?1", [source])?;
        }
        // A child annotation can outlive its disposable cache row.
        let uid = format!("{}:{}", chat.agent, id);
        tx.execute("DELETE FROM library.chat_state WHERE uid=?1", [&uid])?;
        if !uids.contains(&uid) {
            uids.push(uid);
        }
    }
    if let Operation::Files(files) = &plan.operation {
        for source in files::cached_owned_sources(files, &chat.agent, &tx)? {
            let mut statement = tx.prepare("SELECT uid FROM sessions WHERE source=?1")?;
            let owned_uids = statement
                .query_map([&source], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            crate::db::delete_by_source(&tx, &source)?;
            tx.execute("DELETE FROM sources WHERE source=?1", [&source])?;
            for uid in owned_uids {
                tx.execute("DELETE FROM library.chat_state WHERE uid=?1", [&uid])?;
                if !uids.contains(&uid) {
                    uids.push(uid);
                }
            }
        }
    }
    tx.commit()?;
    Ok(uids)
}

fn valid_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id != "."
            && id != ".."
            && !id.chars().any(|c| c.is_control() || c == '/' || c == '\\'),
        "Invalid source session ID"
    );
    Ok(())
}

/// Reject symlinks in every component, not just the last path element.
fn safe_path(root: &Path, path: &Path, directory: bool) -> Result<PathBuf> {
    ensure!(
        root.is_absolute() && path.is_absolute(),
        "Source store must be an absolute path"
    );
    ensure!(
        path.starts_with(root),
        "Source path escapes known harness store: {}",
        path.display()
    );
    let mut p = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, Component::ParentDir | Component::CurDir),
            "Non-normalized source path"
        );
        p.push(part);
        let m =
            std::fs::symlink_metadata(&p).with_context(|| format!("Inspecting {}", p.display()))?;
        // The effective home may sit below an OS alias such as macOS /var -> /private/var.
        // Every component at or below the explicitly validated store boundary must be real.
        ensure!(
            !p.starts_with(root) || !m.file_type().is_symlink(),
            "Refusing symlink {}",
            p.display()
        );
    }
    let m = std::fs::metadata(path)?;
    ensure!(
        if directory { m.is_dir() } else { m.is_file() },
        "Unexpected source file type: {}",
        path.display()
    );
    ensure!(
        path.canonicalize()?.starts_with(root.canonicalize()?),
        "Source resolves outside its known store"
    );
    Ok(path.to_path_buf())
}

fn inventory(root: &Path, path: &Path, stamps: &mut BTreeSet<PathBuf>) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    safe_path(root, path, m.is_dir())?;
    stamps.insert(path.to_path_buf());
    if m.is_dir() {
        for e in std::fs::read_dir(path)? {
            inventory(root, &e?.path(), stamps)?;
        }
    }
    Ok(())
}

fn add_database(root: &Path, path: &Path, stamps: &mut BTreeSet<PathBuf>) -> Result<()> {
    safe_path(root, path, false)?;
    stamps.insert(path.to_path_buf());
    for suffix in ["-wal", "-journal"] {
        let p = PathBuf::from(format!("{}{suffix}", path.to_string_lossy()));
        if p.try_exists()? {
            safe_path(root, &p, false)?;
            stamps.insert(p);
        }
    }
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn require_idle(home: &Path, agent: &str) -> Result<()> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    let system = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    for (pid, process) in system.processes() {
        if pid.as_u32() == std::process::id() {
            continue;
        }
        let name = process.name().to_ascii_lowercase();
        let exe = process
            .exe()
            .map(|p| p.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let cmd = process.cmd();
        let matches = if agent == "cursor" {
            name == "cursor"
                || name == "cursor.exe"
                || Path::new(&exe).file_stem().and_then(|s| s.to_str()) == Some("cursor")
                || name.starts_with("cursor helper")
                || exe.contains("/cursor.app/")
                || exe.contains("\\cursor\\")
        } else {
            let base = Path::new(&exe)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            name == agent
                || base == agent
                || cmd.iter().take(3).any(|s| {
                    let s = s
                        .chars()
                        .map(|c| {
                            if c == '\\' {
                                '/'
                            } else {
                                c.to_ascii_lowercase()
                            }
                        })
                        .collect::<String>();
                    match agent {
                        "omp" => {
                            s.contains("@oh-my-pi/") || s.contains("/omp/") || s.ends_with("/omp")
                        }
                        "pi" => s.contains("@earendil-works/pi-coding-agent") || s.ends_with("/pi"),
                        "claude" => {
                            s.contains("@anthropic-ai/claude-code") || s.ends_with("/claude")
                        }
                        _ => false,
                    }
                })
        };
        if !matches {
            continue;
        }
        if agent == "cursor" {
            bail!("Quit Cursor completely before permanently deleting a chat (process {pid} is running)");
        }
        let env = process.environ();
        let get = |key: &str| {
            env.iter()
                .find_map(|s| s.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
                .filter(|s| !s.is_empty())
        };
        let process_home = get("HOME").or_else(|| get("USERPROFILE"))
            .with_context(|| format!("Cannot determine {agent} process {pid}'s store ownership; stop that harness before deletion"))?;
        let (override_key, rel) = match agent {
            "omp" => ("PI_CODING_AGENT_DIR", ".omp/agent"),
            "pi" => ("PI_CODING_AGENT_DIR", ".pi/agent"),
            "claude" => ("CLAUDE_CONFIG_DIR", ".claude"),
            _ => bail!("Unknown direct-deletion process ownership"),
        };
        let configured = get(override_key);
        let mut store = configured
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(process_home).join(rel));
        if agent == "omp" {
            if configured.is_none() {
                let config = get("PI_CONFIG_DIR").unwrap_or(".omp");
                let mut base = Path::new(process_home).join(config);
                if let Some(profile) = get("OMP_PROFILE").or_else(|| get("PI_PROFILE")) {
                    valid_id(profile).context(
                        "Cannot resolve OMP process profile ownership; stop it before deletion",
                    )?;
                    base = base.join("profiles").join(profile);
                }
                store = base.join("agent");
            }
            // A migrated XDG store changes session ownership; absent paths do not.
            if let Some(data) = get("XDG_DATA_HOME") {
                let mut xdg = Path::new(data).join("omp");
                if let Some(profile) = get("OMP_PROFILE").or_else(|| get("PI_PROFILE")) {
                    xdg = xdg.join("profiles").join(profile);
                }
                if xdg.try_exists()? && configured.is_none() {
                    store = xdg;
                }
            }
        }
        ensure!(store.is_absolute(), "Cannot determine {agent} process {pid}'s relative store override; stop it before deletion");
        let target = home.join(rel).canonicalize()?;
        for argument in cmd {
            let value = argument
                .split_once('=')
                .map(|(_, value)| value)
                .unwrap_or(argument);
            let path = Path::new(value);
            if path.is_absolute() && path.canonicalize().is_ok_and(|p| p.starts_with(&target)) {
                bail!("Stop {agent} process {pid} using the targeted source store before permanent deletion");
            }
        }
        let owner = store.canonicalize().with_context(|| {
            format!(
                "Cannot resolve store ownership of {agent} process {pid}; stop it before deletion"
            )
        })?;
        ensure!(
            owner != target,
            "Stop all {agent} processes sharing {} before permanent deletion (process {pid})",
            target.display()
        );
    }
    Ok(())
}
