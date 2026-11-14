use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");

fn call(root: &Path, action: &str, directories: &[&Path]) -> Value {
    let mut child = Command::new(BIN)
        .arg("--db").arg(root.join("index.sqlite")).arg("library")
        .env("TERN_PROJECTS_HOME", root.join("home"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("PATH", std::env::join_paths(directories).unwrap())
        .env("PATHEXT", ".EXE;.CMD")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(
        json!({"op":"workspaces", "request":{"action":action}}).to_string().as_bytes(),
    ).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn executable(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join(if cfg!(windows) { "ovnfs.exe" } else { "ovnfs" });
    // Not a runnable program: detect must only inspect the executable file.
    fs::write(&path, "not an executable image\n").unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

fn assert_detected(data: Value, expected: &Path) {
    assert_eq!(data["available"], true);
    assert_eq!(Path::new(data["path"].as_str().unwrap()).canonicalize().unwrap(), expected.canonicalize().unwrap());
}

#[test]
fn detection_handles_missing_installed_removed_and_path_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    assert_eq!(call(root, "detect", &[])["available"], false);
    assert_eq!(call(root, "poll", &[])["state"], "missing");
    let official = if cfg!(windows) { root.join("local/Programs/ovnfs") }
        else { root.join("home/.local/bin") };
    let installed = executable(&official);
    assert_detected(call(root, "detect", &[]), &installed);
    let directory = root.join("path with spaces");
    let preferred = executable(&directory);
    assert_detected(call(root, "detect", &[&directory]), &preferred);
    fs::remove_file(preferred).unwrap();
    assert_detected(call(root, "detect", &[&directory]), &installed);
    fs::remove_file(installed).unwrap();
    assert_eq!(call(root, "detect", &[&directory])["available"], false);
}

#[test]
fn an_installed_but_broken_executable_is_an_error_not_missing() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("bin");
    executable(&directory);
    assert_eq!(call(temp.path(), "detect", &[&directory])["available"], true);
    let status = call(temp.path(), "poll", &[&directory]);
    assert_eq!(status["state"], "error");
    assert!(status["error"].as_str().is_some_and(|reason| !reason.is_empty()));
}
