#![cfg(unix)]
use serde_json::{json, Value};
use std::{fs, io::Write, os::unix::fs::PermissionsExt, path::Path, process::{Command, Output, Stdio}};
const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");
fn command(root: &Path, request: Value) -> std::process::Child { command_env(root, request, &[]) }
fn command_env(root: &Path, request: Value, env: &[(&str, &str)]) -> std::process::Child {
    let mut child = Command::new(BIN).args(["--db", root.join("index.sqlite").to_str().unwrap(), "library"])
        .env("PATH", format!("{}:/usr/bin:/bin", root.display())).env("WS_FIXTURE", root).envs(env.iter().copied())
        .env("TERN_PROJECTS_HOME", root.join("home"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(json!({"op":"workspaces", "request":request}).to_string().as_bytes()).unwrap();
    child
}
fn call(root: &Path, request: Value) -> Output { command(root, request).wait_with_output().unwrap() }
fn value(out: Output) -> Value {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("ovnfs"), r##"#!/bin/sh
printf '%s\n' "$*" >> "$WS_FIXTURE/calls"
case "$1" in
 status)
  if test -f "$WS_FIXTURE/stopped"; then echo 'not running' >&2; exit 1; fi
  if test -f "$WS_FIXTURE/error"; then echo 'permission denied' >&2; exit 1; fi
  echo '{"pid":42}';;
 ls) if test -f "$WS_FIXTURE/list-error"; then echo 'listing unavailable' >&2; exit 2; fi; cat "$WS_FIXTURE/list";;
 diff) echo '{"changes":[{"path":"a.txt","status":"M"}]}' ;;
 apply)
  if test "$3" = '--dry-run'; then echo '{"source":"/source","applied":[{"path":"a.txt","status":"M"}],"merged":[],"conflicts":[{"path":"b.txt","markers":true}]}'; exit 1; fi
  touch "$WS_FIXTURE/applying"; sleep 0.3; echo '{"source":"/source","applied":[],"merged":[],"conflicts":[]}' ;;
 *) echo '{}' ;;
esac
"##).unwrap();
    fs::set_permissions(temp.path().join("ovnfs"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(temp.path().join("list"), json!({"checkouts":[
        {"name":"parent", "path":"/mount/parent", "source":"/source", "base":"base", "created":"2026-01-01T00:00:00Z", "touched":"2026-02-01", "git":false},
        {"name":"child", "path":"/mount/child", "source":"/source", "base":"base", "created":"2026-03-01T00:00:00Z", "touched":"2026-03-01", "parent":"snap", "git":false}],
        "snapshots":[{"id":"snap", "checkout":"parent", "created":"2026-02-01T00:00:00Z"}]}).to_string()).unwrap();
    temp
}
#[test]
fn detection_never_runs_ovnfs_and_off_path_operations_use_the_resolved_program() {
    let t = fixture();
    assert_eq!(value(call(t.path(), json!({"action":"detect"})))["available"], true);
    assert!(!t.path().join("calls").exists(), "detection must not spawn OVNFS");
    let installed = t.path().join("home/.local/bin");
    fs::create_dir_all(&installed).unwrap();
    fs::rename(t.path().join("ovnfs"), installed.join("ovnfs")).unwrap();
    let invoke = |request| command_env(t.path(), request, &[("PATH", "/usr/bin:/bin")]).wait_with_output().unwrap();
    assert_eq!(value(invoke(json!({"action":"detect"})))["available"], true);
    assert!(!t.path().join("calls").exists());
    assert_eq!(value(invoke(json!({"action":"poll"})))["state"], "running");
    value(invoke(json!({"action":"start"})));
    assert_eq!(fs::read_to_string(t.path().join("calls")).unwrap(), "status --json\nls --json\n--json start\n");
}

#[test]
fn stopped_and_error_never_list_or_autostart() {
    let t = fixture();
    for (file, state) in [("stopped", "stopped"), ("error", "error")] {
        fs::write(t.path().join(file), "").unwrap();
        assert_eq!(value(call(t.path(), json!({"action":"poll"})))["state"], state);
        fs::remove_file(t.path().join(file)).unwrap();
    }
    assert_eq!(fs::read_to_string(t.path().join("calls")).unwrap(), "status --json\nstatus --json\n");
}
#[test]
fn snapshot_lineage_rejects_reused_parent_and_changed_identity() {
    let t = fixture();
    let first = value(call(t.path(), json!({"action":"poll"})));
    assert_eq!(first["checkouts"][1]["parent_name"], "parent");
    let stale = call(t.path(), json!({"action":"remove","name":"parent","identity":"wrong","expected":"wrong"}));
    assert!(!stale.status.success());
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("rm "));
    let mut list: Value = serde_json::from_str(&fs::read_to_string(t.path().join("list")).unwrap()).unwrap();
    list["checkouts"][0]["created"] = json!("2026-04-01T00:00:00Z");
    fs::write(t.path().join("list"), list.to_string()).unwrap();
    assert!(value(call(t.path(), json!({"action":"poll"})))["checkouts"][1]["parent_name"].is_null());
}
#[test]
fn partial_conflict_preview_and_reload_guard() {
    let t = fixture();
    let poll = value(call(t.path(), json!({"action":"poll"})));
    let identity = &poll["checkouts"][0]["identity"];
    let preview = value(call(t.path(), json!({"action":"preview","name":"parent","identity":identity})));
    assert_eq!(preview["conflicts"][0]["markers"], true);
    let stale = call(t.path(), json!({"action":"apply","name":"parent","identity":identity,"expected":"stale"}));
    assert!(!stale.status.success());
    let applying = command(t.path(), json!({"action":"apply","name":"parent","identity":identity,"expected":preview["token"]}));
    // Wait for the fixture's handshake, not an arbitrary assumption about process scheduling.
    for _ in 0..200 {
        if t.path().join("applying").exists() { break; }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(t.path().join("applying").exists());
    assert_eq!(value(call(t.path(), json!({"action":"poll"})))["busy"], true);
    assert!(!call(t.path(), json!({"action":"remove","name":"parent","identity":identity,"expected":"anything"})).status.success());
    value(applying.wait_with_output().unwrap());
    assert_eq!(value(call(t.path(), json!({"action":"poll"})))["busy"], false);
}

fn git(path: &Path, args: &[&str]) {
    let out = Command::new("/usr/bin/git").arg("-C").arg(path)
        .args(["-c", "user.name=Workspace Test", "-c", "user.email=ws-test@example.invalid"])
        .args(args).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}
fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "--initial-branch=main"]);
}
fn set_checkout(root: &Path, source: &Path, path: &Path, is_git: bool) -> Value {
    let mut list: Value = serde_json::from_str(&fs::read_to_string(root.join("list")).unwrap()).unwrap();
    let mut checkout = list["checkouts"][0].clone();
    checkout["source"] = json!(source);
    checkout["path"] = json!(path);
    checkout["git"] = json!(is_git);
    list["checkouts"] = json!([checkout]);
    fs::write(root.join("list"), list.to_string()).unwrap();
    value(call(root, json!({"action":"poll"})))["checkouts"][0].clone()
}
#[test]
fn repository_selection_environment_is_removed_from_git_and_ovnfs() {
    let t = fixture();
    let source = t.path().join("source");
    let checkout = t.path().join("checkout");
    init_repo(&source);
    git(&source, &["commit", "--allow-empty", "-m", "source"]);
    init_repo(&checkout);
    git(&checkout, &["fetch", source.to_str().unwrap(), "main"]);
    git(&checkout, &["reset", "--hard", "FETCH_HEAD"]);
    let c = set_checkout(t.path(), &source, &checkout, true);
    let overrides = [
        ("GIT_DIR", "/wrong"), ("GIT_WORK_TREE", "/wrong"), ("GIT_INDEX_FILE", "/wrong"),
        ("GIT_OBJECT_DIRECTORY", "/wrong"), ("GIT_ALTERNATE_OBJECT_DIRECTORIES", "/wrong"),
        ("GIT_COMMON_DIR", "/wrong"), ("GIT_NAMESPACE", "wrong"), ("GIT_SHALLOW_FILE", "/wrong"),
        ("GIT_REPLACE_REF_BASE", "refs/wrong"), ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_CEILING_DIRECTORIES", "/"), ("GIT_DISCOVERY_ACROSS_FILESYSTEM", "0"), ("GIT_PREFIX", "/wrong"),
        ("GIT_IMPLICIT_WORK_TREE", "0"), ("GIT_GRAFT_FILE", "/wrong"),
        ("GIT_INTERNAL_SUPER_PREFIX", "/wrong"), ("GIT_QUARANTINE_PATH", "/wrong"),
        ("GIT_CONFIG", "/wrong"), ("GIT_CONFIG_PARAMETERS", "'core.worktree'='/wrong'"),
        ("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "core.worktree"), ("GIT_CONFIG_VALUE_0", "/wrong"),
    ];
    let check = format!("test -z \"{}\" || {{ echo 'repository environment leaked' >&2; exit 93; }}\n",
        overrides.iter().map(|(key, _)| format!("${{{key}+x}}")).collect::<String>());
    let ovnfs = fs::read_to_string(t.path().join("ovnfs")).unwrap();
    fs::write(t.path().join("ovnfs"), ovnfs.replacen("#!/bin/sh\n", &format!("#!/bin/sh\n{check}"), 1)).unwrap();
    fs::write(t.path().join("git"), format!("#!/bin/sh\n{check}exec /usr/bin/git \"$@\"\n")).unwrap();
    fs::set_permissions(t.path().join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let invoke = |request| command_env(t.path(), request, &overrides).wait_with_output().unwrap();
    assert_eq!(value(invoke(json!({"action":"poll"})))["state"], "running");
    let info = value(invoke(json!({"action":"details","name":"parent","identity":c["identity"]})));
    assert!(info["error"].is_null(), "{info}");
    assert_eq!(info["commits"]["count"], 0);
    let existing = invoke(json!({"action":"branch","name":"parent","identity":c["identity"],"branch":"main"}));
    assert!(!existing.status.success());
    assert!(String::from_utf8_lossy(&existing.stderr).contains("Branch already exists"));
    value(invoke(json!({"action":"branch","name":"parent","identity":c["identity"],"branch":"new-branch"})));
    assert_eq!(value(invoke(json!({"action":"roots","paths":[source]})))[source.to_str().unwrap()]["source"], json!(source.canonicalize().unwrap()));
}
#[test]
fn authorization_uses_real_paths_and_current_successful_listing() {
    let t = fixture();
    let checkout = t.path().join("checkout");
    fs::create_dir_all(checkout.join("nested")).unwrap();
    let alias = t.path().join("mount-alias");
    std::os::unix::fs::symlink(&checkout, &alias).unwrap();
    let c = set_checkout(t.path(), t.path(), &alias, false);
    assert_eq!(c["location"], json!(checkout.canonicalize().unwrap()));
    let authorize = |state: &str, live: bool, removing: bool| call(t.path(), json!({
        "action":"authorize","name":"parent","identity":c["identity"],"removing":removing,
        "agents":[{"cwd":checkout.join("nested"),"state":state,"live":live},
                  {"cwd":"/unresolvable-other-agent","state":"working","live":true}]}));
    for state in ["working", "waiting_input"] { assert!(!authorize(state, true, false).status.success()); }
    assert!(!authorize("idle", true, true).status.success());
    value(authorize("idle", true, false));
    value(authorize("done", false, true));
    fs::remove_dir(checkout.join("nested")).unwrap();
    // Deleted descendants retain their lexical association through both spellings.
    for cwd in [checkout.join("nested"), alias.join("nested")] {
        for (state, live, removing) in [("working", true, false), ("waiting_input", true, false), ("idle", true, true)] {
            let denied = call(t.path(), json!({"action":"authorize","name":"parent","identity":c["identity"],"removing":removing,
                "agents":[{"cwd":cwd,"state":state,"live":live}]}));
            assert!(!denied.status.success(), "deleted cwd must block: {}", cwd.display());
        }
    }
    value(call(t.path(), json!({"action":"authorize","name":"parent","identity":c["identity"],"removing":true,
        "agents":[{"cwd":t.path().join("checkout-other/missing"),"state":"working","live":true}]})));
    let info = value(call(t.path(), json!({"action":"details","name":"parent","identity":c["identity"]})));
    fs::write(t.path().join("list-error"), "").unwrap();
    assert!(!authorize("done", false, true).status.success());
    assert!(!call(t.path(), json!({"action":"remove","name":"parent","identity":c["identity"],"expected":info["token"]})).status.success());
    fs::remove_file(t.path().join("list-error")).unwrap();
    fs::write(t.path().join("list"), r#"{"checkouts":[],"snapshots":[]}"#).unwrap();
    assert!(!authorize("done", false, true).status.success());
    assert!(!call(t.path(), json!({"action":"remove","name":"parent","identity":c["identity"],"expected":info["token"]})).status.success());
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("rm "));
}
#[test]
fn unborn_source_counts_checkout_commits_but_corrupt_head_blocks_removal() {
    let t = fixture();
    let source = t.path().join("source");
    let checkout = t.path().join("checkout");
    init_repo(&source);
    init_repo(&checkout);
    let c = set_checkout(t.path(), &source, &checkout, true);
    let info = || value(call(t.path(), json!({"action":"details","name":"parent","identity":c["identity"]})));
    let empty = info();
    assert!(empty["error"].is_null(), "{empty}");
    assert_eq!(empty["commits"]["count"], 0);
    for title in ["first", "second"] { git(&checkout, &["commit", "--allow-empty", "-m", title]); }
    let committed = info();
    assert!(committed["error"].is_null(), "{committed}");
    assert_eq!(committed["commits"]["count"], 2);
    assert!(committed["commits"]["tip"].is_null());
    assert_eq!(committed["commits"]["titles"].as_array().unwrap().len(), 2);
    // Saving is deliberately a separate stage: it must never remove without another authorization.
    let saved = value(call(t.path(), json!({"action":"save-before-remove","name":"parent","identity":c["identity"],"expected":committed["token"],"branch":"saved"})));
    assert!(saved["token"].is_string());
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("rm "));
    assert!(!call(t.path(), json!({"action":"authorize","name":"parent","identity":c["identity"],"removing":true,
        "agents":[{"cwd":checkout,"state":"working","live":true}]})).status.success());
    fs::write(source.join(".git/HEAD"), "unreadable HEAD\n").unwrap();
    let broken = info();
    assert!(broken["error"].is_string(), "{broken}");
    let removal = call(t.path(), json!({"action":"remove","name":"parent","identity":c["identity"],"expected":broken["token"]}));
    assert!(!removal.status.success());
    assert!(String::from_utf8_lossy(&removal.stderr).contains("Cannot safely check"));
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("rm "));
}
#[test]
fn unborn_checkout_counts_other_refs_and_blocks_unreadable_history() {
    let t = fixture();
    let source = t.path().join("source");
    let checkout = t.path().join("checkout");
    init_repo(&source);
    git(&source, &["commit", "--allow-empty", "-m", "source base"]);
    init_repo(&checkout);
    let c = set_checkout(t.path(), &source, &checkout, true);
    let info = || value(call(t.path(), json!({"action":"details","name":"parent","identity":c["identity"]})));
    // Even a source with commits does not make a truly empty checkout nonempty.
    assert_eq!(info()["commits"]["count"], 0);
    git(&checkout, &["fetch", source.to_str().unwrap(), "main"]);
    git(&checkout, &["reset", "--hard", "FETCH_HEAD"]);
    for title in ["private first", "private second"] { git(&checkout, &["commit", "--allow-empty", "-m", title]); }
    git(&checkout, &["tag", "retained-tag"]);
    git(&checkout, &["switch", "--orphan", "empty-head"]);
    let orphan = info();
    assert!(orphan["error"].is_null(), "{orphan}");
    assert!(orphan["commits"]["head"].is_null());
    assert_eq!(orphan["commits"]["count"], 2);
    let titles = orphan["commits"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 2);
    assert!(titles.iter().any(|title| title.as_str().unwrap().ends_with("private second")));
    assert!(!titles.iter().any(|title| title.as_str().unwrap().ends_with("source base")));
    assert!(orphan["commits"]["refs"].as_array().unwrap().contains(&json!("refs/heads/main")));
    let save = call(t.path(), json!({"action":"save-before-remove","name":"parent","identity":c["identity"],"expected":orphan["token"],"branch":"saved"}));
    assert!(!save.status.success());
    assert!(String::from_utf8_lossy(&save.stderr).contains("Switch the workspace to a retained branch"));
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("apply parent --branch"));
    // A tag alone also keeps the history alive, without counting duplicate refs.
    git(&checkout, &["branch", "-D", "main"]);
    assert_eq!(info()["commits"]["count"], 2);
    fs::write(checkout.join(".git/refs/heads/broken"), "1111111111111111111111111111111111111111\n").unwrap();
    let broken = info();
    assert!(broken["error"].is_string(), "{broken}");
    let removal = call(t.path(), json!({"action":"remove","name":"parent","identity":c["identity"],"expected":broken["token"]}));
    assert!(!removal.status.success());
    assert!(String::from_utf8_lossy(&removal.stderr).contains("Cannot safely check"));
    assert!(!fs::read_to_string(t.path().join("calls")).unwrap().contains("rm "));
}
#[test]
fn subfolder_capture_root_is_explicit_and_validated_before_creation() {
    let t = fixture();
    let source = t.path().join("source");
    init_repo(&source);
    let folder = source.join("app");
    fs::create_dir(&folder).unwrap();
    let roots = value(call(t.path(), json!({"action":"roots","paths":[folder]})));
    assert_eq!(roots[folder.to_str().unwrap()]["source"], json!(source.canonicalize().unwrap()));
    assert!(!call(t.path(), json!({"action":"new","source":folder,"name":"test"})).status.success());
    value(call(t.path(), json!({"action":"new","source":source,"name":"test"})));
    let calls = fs::read_to_string(t.path().join("calls")).unwrap();
    assert!(!calls.contains(&format!("new {} ", folder.display())));
    assert!(calls.contains(&format!("new {} --name test --json", source.display())));
}
