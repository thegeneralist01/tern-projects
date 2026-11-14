//! OVNFS is authoritative: no stored checkout list, automatic daemon startup or operation replay.
use anyhow::{bail, ensure, Context, Result};
use crate::text::plain_path;
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: String,
    pub name: Option<String>,
    pub source: Option<String>,
    pub identity: Option<String>,
    pub expected: Option<String>,
    pub branch: Option<String>,
    pub agents: Option<Vec<Agent>>,
    pub removing: Option<bool>,
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub cwd: Option<String>,
    pub state: String,
    pub live: bool,
}

// Discovery performs filesystem checks only: opening Projects must not start OVNFS.
fn executable(home: &Path) -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).filter(|p| !p.as_os_str().is_empty()).collect())
        .unwrap_or_default();
    directories.push(home.join(".local/bin"));
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        directories.push(PathBuf::from(local).join("Programs/ovnfs"));
    }
    find_executable(&directories)
}
fn find_executable(directories: &[PathBuf]) -> Option<PathBuf> {
    let names = crate::harnesses::executable_names("ovnfs");
    for directory in directories {
        for name in &names {
            let path = directory.join(name);
            if crate::harnesses::executable_file(&path) {
                // Preserve the invoked name, as harness discovery does.
                return if path.is_absolute() { Some(path) }
                    else { std::env::current_dir().ok().map(|cwd| cwd.join(path)) };
            }
        }
    }
    None
}

fn command(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    // OVNFS also spawns Git. Never let the launching shell select another repository,
    // index, object store or ref namespace, including through command-scoped config.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("GIT_CONFIG") || matches!(name.as_ref(),
            "GIT_DIR" | "GIT_WORK_TREE" | "GIT_INDEX_FILE" | "GIT_OBJECT_DIRECTORY"
            | "GIT_ALTERNATE_OBJECT_DIRECTORIES" | "GIT_COMMON_DIR" | "GIT_NAMESPACE"
            | "GIT_SHALLOW_FILE" | "GIT_REPLACE_REF_BASE" | "GIT_NO_REPLACE_OBJECTS"
            | "GIT_CEILING_DIRECTORIES" | "GIT_DISCOVERY_ACROSS_FILESYSTEM" | "GIT_PREFIX"
            | "GIT_IMPLICIT_WORK_TREE" | "GIT_GRAFT_FILE" | "GIT_INTERNAL_SUPER_PREFIX" | "GIT_QUARANTINE_PATH") {
            command.env_remove(key);
        }
    }
    command
}
fn run(program: &Path, args: &[&str]) -> Result<Output> {
    command(program, args).output().with_context(|| format!("cannot run {}", program.display()))
}
fn output(out: Output, conflicts: bool) -> Result<Value> {
    let value = serde_json::from_slice::<Value>(&out.stdout);
    if out.status.success() || (conflicts && out.status.code() == Some(1)
        && value.as_ref().ok().and_then(|v| v.get("conflicts")).is_some_and(Value::is_array)) {
        return value.context("invalid OVNFS JSON");
    }
    bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
}
fn ovnfs(executable: &Path, args: &[&str], conflicts: bool) -> Result<Value> { output(run(executable, args)?, conflicts) }
fn git_output(path: &str, args: &[&str]) -> Result<Output> {
    command("git", &["-C", path]).args(args).output().context("cannot run git")
}
fn git(path: &str, args: &[&str]) -> Result<String> {
    let out = git_output(path, args)?;
    ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}
fn real_path(path: &str) -> Result<String> {
    let path = plain_path(std::fs::canonicalize(path)?);
    let text = path.to_str().context("path is not Unicode")?;
    Ok(if cfg!(windows) { text.replace('\\', "/") } else { text.to_owned() })
}
// Deleted agent cwds still belong to their checkout, even through macOS /var aliases.
fn agent_location(path: &str) -> Option<String> {
    if let Ok(location) = real_path(path) { return Some(location); }
    for ancestor in Path::new(path).ancestors().skip(1) {
        if let Ok(root) = real_path(ancestor.to_str()?) {
            let suffix = Path::new(path).strip_prefix(ancestor).ok()?;
            let location = Path::new(&root).join(suffix);
            let text = location.to_str()?;
            return Some(if cfg!(windows) { text.replace('\\', "/") } else { text.to_owned() });
        }
    }
    None
}
fn capture_root(source: &str) -> Result<String> {
    let source = real_path(source)?;
    ensure!(Path::new(&source).is_dir(), "source is not a directory");
    let out = git_output(&source, &["rev-parse", "--show-toplevel"])?;
    if out.status.success() {
        real_path(String::from_utf8(out.stdout)?.trim())
    } else {
        Ok(source) // Non-Git folders are supported by OVNFS.
    }
}
fn hash(value: &Value) -> String { format!("{:x}", Sha256::digest(value.to_string().as_bytes())) }
fn identity(c: &Value) -> String { hash(&json!([c["name"], c["source"], c["path"], c["created"], c["base"]])) }
fn lock_file(db: &Path) -> Result<File> {
    if let Some(parent) = db.parent() { std::fs::create_dir_all(parent)?; }
    Ok(OpenOptions::new().write(true).create(true).truncate(false).open(db.with_extension("workspaces.lock"))?)
}
fn busy(db: &Path) -> Result<bool> {
    let file = lock_file(db)?;
    match FileExt::try_lock_shared(&file) {
        Ok(()) => Ok(false),
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Ok(true),
        Err(e) => Err(e.into()),
    }
}
fn status(executable: Option<&Path>) -> Result<Value> {
    let Some(executable) = executable else {
        return Ok(json!({"state":"missing"}));
    };
    let out = match run(executable, &["status", "--json"]) {
        Ok(out) => out,
        Err(e) => return Ok(json!({"state":"error", "error":format!("{e:#}")})),
    };
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if out.status.code() == Some(1) && out.stdout.is_empty() && stderr == "not running" {
        return Ok(json!({"state":"stopped", "error":"OVNFS is not running."}));
    }
    if !out.status.success() { return Ok(json!({"state":"error", "error":stderr})); }
    let mut value = output(out, false)?;
    value["state"] = json!("running");
    Ok(value)
}
// Go's RFC3339Nano omits trailing fractional zeroes, so raw strings do not sort chronologically.
fn timestamp_key(value: &Value) -> Option<(&str, u32)> {
    let text = value.as_str()?.strip_suffix('Z')?;
    let (date, fraction) = text.split_once('.').unwrap_or((text, ""));
    if date.len() != 19 || fraction.len() > 9 { return None; }
    let mut nanos = 0;
    for digit in fraction.bytes() {
        if !digit.is_ascii_digit() { return None; }
        nanos = nanos * 10 + u32::from(digit - b'0');
    }
    Some((date, nanos * 10u32.pow(9 - fraction.len() as u32)))
}
fn listing(executable: &Path) -> Result<Value> {
    let mut data = ovnfs(executable, &["ls", "--json"], false)?;
    let snapshots = data.as_object_mut().context("OVNFS listing must be an object")?
        .remove("snapshots").context("OVNFS snapshots missing")?;
    let snapshots = snapshots.as_array().context("OVNFS snapshots must be an array")?;
    let checkouts = data["checkouts"].as_array_mut().context("OVNFS checkouts missing")?;
    for c in checkouts.iter_mut() {
        c["identity"] = json!(identity(c));
        c["location"] = json!(c["path"].as_str().and_then(|p| real_path(p).ok()));
        c["source_location"] = json!(c["source"].as_str().and_then(|p| real_path(p).ok()));
    }
    // Snapshot names are not identities: a removed parent's reused name must not adopt its children.
    let parents: Vec<_> = checkouts.iter().map(|c| {
        let snap = snapshots.iter().find(|s| !c["parent"].is_null() && s["id"] == c["parent"])?;
        let parent = checkouts.iter().find(|p| p["name"] == snap["checkout"] && p["name"] != c["name"]
            && p["source"] == c["source"] && timestamp_key(&p["created"]).zip(timestamp_key(&snap["created"])).is_some_and(|(a, b)| a <= b))?;
        Some(parent["name"].clone())
    }).collect();
    for (c, parent) in checkouts.iter_mut().zip(parents) {
        if let Some(name) = parent { c["parent_name"] = name; }
    }
    Ok(data)
}
fn checkout(executable: &Path, req: &Request) -> Result<Value> {
    let name = req.name.as_deref().context("workspace name required")?;
    ensure!(status(Some(executable))?["state"] == "running", "OVNFS is not running; refresh its status first");
    let data = listing(executable)?;
    let c = data["checkouts"].as_array().unwrap().iter().find(|c| c["name"] == name)
        .context("workspace no longer exists; refresh")?;
    ensure!(req.identity.as_deref() == c["identity"].as_str(), "workspace identity changed; refresh");
    Ok(c.clone())
}
// A missing symbolic branch is unborn; unreadable HEADs, refs and objects are errors.
fn head(path: &str) -> Result<Option<String>> {
    let out = git_output(path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
    if out.status.success() { return Ok(Some(String::from_utf8(out.stdout)?.trim().to_owned())); }
    ensure!(out.status.code() == Some(1), "Cannot read Git HEAD: {}", String::from_utf8_lossy(&out.stderr).trim());
    ensure!(git(path, &["rev-parse", "--is-inside-work-tree"])? == "true", "Not a readable Git work tree");
    let reference = git(path, &["symbolic-ref", "--quiet", "HEAD"])?;
    ensure!(reference.starts_with("refs/heads/"), "HEAD does not name a branch");
    let reference = git_output(path, &["show-ref", "--verify", "--quiet", &reference])?;
    ensure!(reference.status.code() == Some(1), "Git HEAD exists but its commit cannot be read");
    Ok(None)
}
fn commits(c: &Value) -> Result<Value> {
    if c["git"] != true { return Ok(Value::Null); }
    let source = c["source"].as_str().context("source missing")?;
    let path = c["path"].as_str().context("workspace path missing")?;
    let tip = head(source)?;
    let head = head(path)?;
    let (count, titles) = if head.is_some() {
        let range = tip.as_ref().map_or_else(|| "HEAD".to_owned(), |tip| format!("{tip}..HEAD"));
        let count: u64 = git(path, &["rev-list", "--count", &range])?.parse()?;
        let log = git(path, &["log", "-25", "--format=%h %s", &range])?;
        (count, log.lines().map(str::to_owned).collect::<Vec<_>>())
    } else {
        // An orphan HEAD can coexist with commits on other refs. Only a genuinely
        // empty ref graph is empty; unreadable refs/objects must block removal.
        let total: u64 = git(path, &["rev-list", "--count", "--all"])?.parse()?;
        if total == 0 { (0, Vec::new()) } else {
            let mut args = Vec::with_capacity(6);
            args.extend(["rev-list", "--count", "--all"]);
            if let Some(tip) = tip.as_deref() { args.extend(["--not", tip]); }
            let count: u64 = if tip.is_none() { total } else { git(path, &args)?.parse()? };
            args[0] = "log"; args[1] = "--format=%h %s"; args.insert(2, "-25");
            let log = git(path, &args)?;
            (count, log.lines().map(str::to_owned).collect::<Vec<_>>())
        }
    };
    let refs = if head.is_none() && count > 0 {
        git(path, &["for-each-ref", "--format=%(refname)"])?.lines().map(str::to_owned).collect::<Vec<_>>()
    } else { Vec::new() };
    Ok(json!({"count":count, "tip":tip, "head":head, "titles":titles, "refs":refs}))
}
fn details(executable: &Path, c: &Value) -> Result<Value> {
    let diff = ovnfs(executable, &["diff", c["name"].as_str().unwrap(), "--json"], false)?;
    let (commits, error) = match commits(c) {
        Ok(v) => (v, Value::Null),
        Err(e) => (Value::Null, json!(format!("Cannot count commits: {e:#}"))),
    };
    let token = hash(&json!([c["identity"], c["touched"], diff, commits]));
    Ok(json!({"changes":diff["changes"], "commits":commits, "error":error, "token":token}))
}
fn preview(executable: &Path, c: &Value) -> Result<Value> {
    let mut result = ovnfs(executable, &["apply", c["name"].as_str().unwrap(), "--dry-run", "--json"], true)?;
    let info = details(executable, c)?;
    result["token"] = json!(hash(&json!([info["token"], result])));
    result["details"] = info;
    Ok(result)
}
fn validate_name(name: &str) -> Result<()> {
    ensure!(name.len() <= 64 && name.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
        "Use 1–64 lowercase letters, numbers, dots, underscores or hyphens; start with a letter or number");
    ensure!(name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)), "Invalid workspace name");
    Ok(())
}
fn save_branch(executable: &Path, c: &Value, branch: &str) -> Result<Value> {
    ensure!(c["git"] == true, "This workspace is not a Git repository");
    ensure!(!branch.starts_with('-') && !branch.is_empty(), "A new branch name is required");
    let source = c["source"].as_str().unwrap();
    git(source, &["check-ref-format", "--branch", branch])?;
    let reference = format!("refs/heads/{branch}");
    let exists = git_output(source, &["show-ref", "--verify", "--quiet", &reference])?;
    ensure!(exists.status.code() == Some(1), "Branch already exists or cannot be checked; choose a new branch");
    ovnfs(executable, &["apply", c["name"].as_str().unwrap(), "--branch", branch, "--json"], false)
}

fn authorize(executable: &Path, req: &Request) -> Result<Value> {
    // Always use a successful current listing, not the panel's last (possibly failed) poll.
    let mut c = checkout(executable, req)?;
    let raw = c["path"].as_str().context("workspace path missing")?;
    let root = real_path(raw).context("Cannot resolve workspace path")?;
    let mut locations = serde_json::Map::new();
    for agent in req.agents.as_ref().context("Fresh Tern agent snapshot required")? {
        let Some(cwd) = agent.cwd.as_deref() else { continue; };
        let location = agent_location(cwd);
        let inside = if let Some(path) = location.as_deref() {
            Path::new(path).starts_with(&root)
        } else {
            // Unrelated unresolved cwds are harmless, but a known descendant of
            // either spelling still belongs here after deletion/inaccessibility.
            Path::new(cwd).starts_with(raw) || Path::new(cwd).starts_with(&root)
        };
        locations.insert(cwd.to_owned(), location.map_or(Value::Bool(false), Value::String));
        if inside && (agent.state == "working" || agent.state == "waiting_input"
            || (req.removing == Some(true) && agent.live)) {
            bail!("{}", if req.removing == Some(true) {
                "An agent is still live here. Close its tab before removing this workspace."
            } else { "An agent is working or waiting for you here. Finish or stop it before applying." });
        }
    }
    c["agent_locations"] = Value::Object(locations);
    Ok(c)
}
fn removal_details(executable: &Path, req: &Request, c: &Value) -> Result<Value> {
    let info = details(executable, c)?;
    ensure!(req.expected.as_deref() == info["token"].as_str(), "Workspace changed; cancel and review removal again");
    ensure!(info["error"].is_null(), "Cannot safely check unreturned commits; fix the Git error first");
    Ok(info)
}

pub fn dispatch(db: &Path, home: &Path, req: Request) -> Result<Value> {
    if req.action == "detect" {
        let path = executable(home).map(plain_path);
        return Ok(json!({"available":path.is_some(), "path":path}));
    }
    if req.action == "resolve" || req.action == "roots" {
        let mut result = serde_json::Map::new();
        for path in req.paths.as_ref().context("paths required")? {
            if result.contains_key(path) { continue; }
            let value = if req.action == "resolve" {
                json!(agent_location(path))
            } else {
                match capture_root(path) {
                    Ok(source) => json!({"source":source}),
                    Err(error) => json!({"error":format!("{error:#}")}),
                }
            };
            result.insert(path.clone(), value);
        }
        return Ok(Value::Object(result));
    }
    let executable = executable(home);
    if req.action == "poll" {
        let mut state = status(executable.as_deref())?;
        state["busy"] = json!(busy(db)?);
        if state["state"] == "running" {
            match listing(executable.as_deref().context("OVNFS is not installed")?) {
                Ok(data) => { state["checkouts"] = data["checkouts"].clone(); },
                Err(e) => { state["state"] = json!("error"); state["error"] = json!(format!("{e:#}")); },
            }
        }
        return Ok(state);
    }
    let executable = executable.as_deref().context("OVNFS is not installed")?;
    if req.action == "authorize" { return authorize(executable, &req); }
    if req.action == "details" { return details(executable, &checkout(executable, &req)?); }
    if req.action == "preview" { return preview(executable, &checkout(executable, &req)?); }
    // The OS-held guard outlives a plugin reload with its CLI child. No journal and no replay.
    let guard = lock_file(db)?;
    guard.try_lock_exclusive().context("A workspace operation is still running; refresh before trying again")?;
    if req.action == "start" { return ovnfs(executable, &["--json", "start"], false); }
    if req.action == "new" {
        let name = req.name.as_deref().context("workspace name required")?;
        validate_name(name)?;
        ensure!(status(Some(executable))?["state"] == "running", "Start OVNFS first");
        let source = req.source.as_deref().context("source folder required")?;
        ensure!(Path::new(source).is_absolute(), "Choose an absolute source folder");
        ensure!(capture_root(source)? == real_path(source)?, "Capture root changed; reopen New workspace to review its actual target");
        return ovnfs(executable, &["new", source, "--name", name, "--json"], false);
    }
    let c = checkout(executable, &req)?;
    let name = c["name"].as_str().unwrap();
    match req.action.as_str() {
        "fork" => {
            let child = req.branch.as_deref().context("fork name required")?;
            validate_name(child)?;
            ovnfs(executable, &["fork", name, "--name", child, "--json"], false)
        },
        "apply" => {
            let current = preview(executable, &c)?;
            ensure!(req.expected.as_deref() == current["token"].as_str(), "Preview changed; cancel and preview again");
            ovnfs(executable, &["apply", name, "--json"], true)
        },
        "save-before-remove" => {
            let info = removal_details(executable, &req, &c)?;
            ensure!(info["commits"]["head"].is_string(),
                "The checked-out branch has no commits yet. Switch the workspace to a retained branch containing these commits, then use Save branch.");
            save_branch(executable, &c, req.branch.as_deref().context("new branch required")?)?;
            let current = checkout(executable, &req)?;
            let after = details(executable, &current)?;
            ensure!(after["error"].is_null() && info["changes"] == after["changes"] && info["commits"] == after["commits"],
                "Workspace changed while saving the branch; review removal again. The branch was saved.");
            // The frontend must obtain another fresh agent authorization before requesting remove.
            Ok(json!({"token":after["token"]}))
        },
        "remove" => {
            removal_details(executable, &req, &c)?;
            ovnfs(executable, &["rm", name, "--json"], false)
        },
        "branch" => save_branch(executable, &c, req.branch.as_deref().context("new branch required")?),
        _ => bail!("unknown workspace operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_bounded_and_not_options() {
        for n in ["hello", "a._-9", "0"] { assert!(validate_name(n).is_ok()); }
        for n in ["", "-flag", ".hidden", "UPPER", "with space", "a/b", &"a".repeat(65)] { assert!(validate_name(n).is_err()); }
    }
    #[test]
    fn fractional_timestamps_compare_chronologically() {
        let a = json!("2026-11-12T00:00:00.12Z");
        let b = json!("2026-11-12T00:00:00.123Z");
        assert!(timestamp_key(&a) < timestamp_key(&b));
        assert_eq!(timestamp_key(&a), timestamp_key(&json!("2026-11-12T00:00:00.120000000Z")));
        assert!(timestamp_key(&json!("malformed")).is_none());
    }
    #[test]
    fn conflict_exit_is_a_successful_result_not_a_retry() {
        #[cfg(unix)] {
            use std::os::unix::process::ExitStatusExt;
            let out = Output {status: std::process::ExitStatus::from_raw(256), stdout: br#"{"conflicts":[{"path":"a","markers":true}]}"#.to_vec(), stderr: b"has conflicts".to_vec()};
            assert_eq!(output(out, true).unwrap()["conflicts"][0]["path"], "a");
        }
    }
    #[test]
    fn guard_survives_a_new_request() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("index.sqlite");
        let guard = lock_file(&db).unwrap();
        guard.try_lock_exclusive().unwrap();
        assert!(busy(&db).unwrap());
        drop(guard);
        assert!(!busy(&db).unwrap());
    }
    #[test]
    fn discovery_checks_executable_files_in_directory_order() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let installed = temp.path().join(".local/bin");
        std::fs::create_dir_all(&installed).unwrap();
        let path = installed.join(if cfg!(windows) { "ovnfs.exe" } else { "ovnfs" });
        std::fs::write(&path, b"fixture executable; discovery must not run it").unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert!(find_executable(std::slice::from_ref(&installed)).is_none());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(find_executable(&[missing, installed]).unwrap().canonicalize().unwrap(), path.canonicalize().unwrap());
    }
}
