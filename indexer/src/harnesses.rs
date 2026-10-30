//! Verified local CLI discovery and fresh-chat command planning.
//!
//! This module never starts an interactive chat. Help probes are bounded and a
//! launch plan must still be accepted by the panel before the host opens a tab.

use crate::text::plain_path;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_BYTES: u64 = 512 * 1024;
const OMP_OVERLAY: &[u8] = b"autoResume: false\n";
const HARNESSES: [(&str, &str); 6] = [
    ("omp", "OMP"),
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("pi", "Pi"),
    ("opencode", "OpenCode"),
    ("cursor", "Cursor Agent"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    resolved: PathBuf,
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl Fingerprint {
    fn read(path: &Path) -> Result<Self> {
        let resolved = fs::canonicalize(path)
            .with_context(|| format!("Cannot resolve executable {}", path.display()))?;
        let metadata = fs::metadata(&resolved)?;
        if !metadata.is_file() || !executable_file(&resolved) {
            bail!("{} is not an executable file", path.display());
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            resolved,
            len: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

/// An executable whose own help identified the requested harness. Discovery is
/// deliberately request-local: a launch never trusts an earlier picker result.
#[derive(Clone, Debug)]
pub struct Executable {
    pub id: String,
    pub path: PathBuf,
    pub help: String,
    home: PathBuf,
    fingerprint: Fingerprint,
}

impl Executable {
    /// Match option declarations, not arbitrary prose mentioning another flag.
    pub fn supports_flag(&self, flag: &str) -> bool {
        help_has_flag(&self.help, flag)
    }

    /// Probe a named subcommand using argv, never a shell. Returning the root
    /// help for an unknown command is not evidence of subcommand support.
    pub fn verify_subcommand(&self, subcommand: &[&str]) -> Result<String> {
        if subcommand.is_empty()
            || subcommand
                .iter()
                .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        {
            bail!("Invalid help subcommand");
        }
        self.ensure_unchanged()?;
        let mut args = subcommand.to_vec();
        args.push("--help");
        let help = probe(&self.path, &self.home, &self.id, &args)?;
        self.ensure_unchanged()?;
        let expected = format!("{} {}", self.id, subcommand.join(" ")).to_lowercase();
        if !help
            .to_lowercase()
            .lines()
            .any(|line| line.contains(&expected))
        {
            bail!("{} does not advertise {}", self.path.display(), expected);
        }
        Ok(help)
    }

    fn ensure_unchanged(&self) -> Result<()> {
        if Fingerprint::read(&self.path)? != self.fingerprint {
            bail!(
                "{} changed during capability detection; request a new launch plan",
                self.path.display()
            );
        }
        Ok(())
    }

    fn multi_root(&self) -> bool {
        matches!(self.id.as_str(), "omp" | "claude" | "codex" | "cursor")
            && self.supports_flag("--add-dir")
    }
}

/// Return a verified CLI, trying PATH and the documented user bin directories.
/// An existing but unidentifiable command is an error, not an installed harness.
pub fn discover(home: &Path, id: &str) -> Result<Option<Executable>> {
    let names: &[&str] = match id {
        "omp" => &["omp"],
        "claude" => &["claude"],
        "codex" => &["codex"],
        "pi" => &["pi"],
        "opencode" => &["opencode"],
        "cursor" => &["cursor-agent", "agent"],
        _ => bail!("Unknown harness: {id}"),
    };
    let mut failures = Vec::new();
    for path in candidates(home, names, true) {
        let result = (|| -> Result<Executable> {
            safe_text(path_text(&path)?, "Executable path")?;
            let fingerprint = Fingerprint::read(&path)?;
            let help = probe(&path, home, id, &["--help"])?;
            if !identifies(id, &help) {
                bail!("help does not identify {}", label(id));
            }
            let executable = Executable {
                id: id.to_owned(),
                path,
                help,
                home: home.to_path_buf(),
                fingerprint,
            };
            executable.ensure_unchanged()?;
            Ok(executable)
        })();
        match result {
            Ok(executable) => return Ok(Some(executable)),
            Err(error) => failures.push(format!("{error:#}")),
        }
    }
    if failures.is_empty() {
        Ok(None)
    } else {
        bail!("{} is unavailable: {}", label(id), failures.join("; "))
    }
}

/// Native deletion callers get the same identity checks as launch callers and
/// must not fall back to raw SQL if the installed command is unsupported.
pub fn native_delete_executable(home: &Path, harness: &str) -> Result<PathBuf> {
    let executable = discover(home, harness)?
        .with_context(|| format!("Install {} to enable permanent deletion", label(harness)))?;
    match harness {
        "codex" => {
            let help = executable
                .verify_subcommand(&["delete"])
                .context("Update Codex to enable permanent deletion")?;
            if !help_has_flag(&help, "--force") || !help.to_lowercase().contains("uuid") {
                bail!("Update Codex to enable permanent deletion: UUID-only --force is required");
            }
        }
        "opencode" => {
            let help = executable
                .verify_subcommand(&["session", "delete"])
                .context("Update OpenCode to enable permanent deletion")?;
            if !help.contains("<sessionID>") || !help.to_lowercase().contains("delete a session") {
                bail!(
                    "Update OpenCode to enable permanent deletion: session delete is unsupported"
                );
            }
        }
        _ => bail!(
            "{} has no supported native deletion command",
            label(harness)
        ),
    }
    executable.ensure_unchanged()?;
    Ok(executable.path)
}

pub fn list(home: &Path) -> Result<Value> {
    let harnesses: Vec<Value> = HARNESSES.iter().map(|&(id, name)| {
        match discover(home, id) {
            Ok(Some(executable)) => {
                let multi_root = executable.multi_root();
                json!({
                    "id": id, "label": name, "path": executable.path,
                    "installed": true, "multi_root": multi_root,
                    "reason": if multi_root { None } else { Some("Additional folders are not supported by this detected CLI; primary-folder-only launch requires confirmation.") }
                })
            }
            Ok(None) => json!({
                "id": id, "label": name, "path": null, "installed": false,
                "multi_root": false, "reason": format!("{} CLI was not found in PATH or the supported installation directories", name)
            }),
            Err(error) => json!({
                "id": id, "label": name, "path": null, "installed": false,
                "multi_root": false, "reason": error.to_string()
            }),
        }
    }).collect();
    Ok(json!({"harnesses": harnesses}))
}

/// Build a fresh interactive command from resolved project folders. Position
/// zero is always Primary. Missing extras are explicit warnings, never silently
/// omitted; callers must confirm any warnings before executing the plan.
pub fn launch(db_path: &Path, home: &Path, harness: &str, folders: &[String]) -> Result<Value> {
    let primary = folders
        .first()
        .context("Choose a primary source folder before starting a new chat")?;
    for folder in folders {
        safe_text(folder, "Source folder")?;
        if !Path::new(folder).is_absolute() {
            bail!("Source folder must be an absolute path: {folder}");
        }
    }
    let cwd = readable_directory(primary)
        .with_context(|| format!("Primary source folder is missing or unreadable: {primary}"))?;
    let cwd = path_text(&cwd)?.to_owned();
    safe_text(&cwd, "Primary source folder")?;
    let executable = discover(home, harness)?
        .with_context(|| format!("{} CLI is not installed", label(harness)))?;
    let mut warnings = Vec::new();
    let mut extras = Vec::new();
    let mut seen = HashSet::from([cwd.clone()]);
    for folder in &folders[1..] {
        match readable_directory(folder) {
            Ok(path) => {
                let normalized = path_text(&path)?.to_owned();
                safe_text(&normalized, "Additional source folder")?;
                if seen.insert(normalized.clone()) {
                    extras.push(normalized);
                }
            }
            Err(error) => warnings.push(format!("Additional source folder was not added because it is missing or unreadable: {folder} ({error})")),
        }
    }
    let mut argv = vec![path_text(&executable.path)?.to_owned()];
    match harness {
        "omp" => {
            if !executable.supports_flag("--cwd") || !executable.supports_flag("--config") {
                bail!(
                    "Update OMP to start a guaranteed fresh chat: --cwd and --config are required"
                );
            }
            let overlay = omp_overlay(db_path)?;
            argv.extend([
                "--cwd".to_owned(),
                cwd.clone(),
                "--config".to_owned(),
                overlay,
            ]);
        }
        "codex" => {
            if !executable.supports_flag("--cd") {
                bail!("Update Codex to start a chat with an explicit primary folder (--cd)");
            }
            argv.extend(["--cd".to_owned(), cwd.clone()]);
        }
        "opencode" => argv.push(cwd.clone()),
        "claude" | "pi" | "cursor" => {}
        _ => bail!("Unknown harness: {harness}"),
    }
    if executable.multi_root() {
        if harness == "claude" && !extras.is_empty() {
            argv.push("--add-dir".to_owned());
            argv.extend(extras);
        } else {
            for folder in extras {
                argv.extend(["--add-dir".to_owned(), folder]);
            }
        }
    } else if !extras.is_empty() {
        for folder in extras {
            warnings.push(format!("Additional source folder was not added: {folder}"));
        }
        warnings.push(format!("{} will open the primary folder only. Additional folders were not added; no native add-directory command is available in this detected version.", label(harness)));
    }
    // CLI --help does not establish an interactive slash-command contract.
    // Do not infer /add or /add-dir from a startup option or another harness.
    let manual_commands: Vec<String> = Vec::new();
    for argument in &argv {
        safe_text(argument, "Launch argument")?;
    }
    executable.ensure_unchanged()?;
    let command = launch_command(&argv, home)?;
    Ok(json!({"cwd": cwd, "argv": argv, "command": command,
        "warnings": warnings, "manual_commands": manual_commands}))
}

fn label(id: &str) -> &str {
    HARNESSES
        .iter()
        .find(|&&(candidate, _)| candidate == id)
        .map(|&(_, label)| label)
        .unwrap_or(id)
}

fn readable_directory(path: &str) -> Result<PathBuf> {
    let path = plain_path(fs::canonicalize(path)?);
    if !path.is_dir() {
        bail!("not a directory");
    }
    fs::read_dir(&path).context("directory cannot be read")?;
    Ok(path)
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("Path cannot be represented as UTF-8: {}", path.display()))
}

fn safe_text(value: &str, name: &str) -> Result<()> {
    if value.is_empty() || value.contains(['\0', '\n', '\r']) {
        bail!("{name} must not be empty or contain NUL/newline characters");
    }
    Ok(())
}

fn help_has_flag(help: &str, flag: &str) -> bool {
    help.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('-')
            && line
                .split_whitespace()
                .any(|word| word.trim_end_matches(',').split('=').next() == Some(flag))
    })
}

fn identifies(id: &str, help: &str) -> bool {
    let header = help
        .lines()
        .take(40)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    match id {
        "omp" => header
            .lines()
            .any(|line| line.starts_with("omp v") || line.trim() == "oh my pi"),
        "claude" => header.contains("claude code") && header.contains("usage: claude"),
        "codex" => header.contains("codex cli") && header.contains("usage: codex"),
        "pi" => header.contains("pi - ai coding assistant") && header.contains("pi [options]"),
        "opencode" => header.contains("opencode [project]") && header.contains("start opencode"),
        "cursor" => {
            !header.contains("grok")
                && (header.contains("cursor agent") || header.contains("cursor cli"))
        }
        _ => false,
    }
}

fn candidates(home: &Path, names: &[&str], user_dirs: bool) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if user_dirs {
        directories.extend(
            [
                ".local/bin",
                ".cargo/bin",
                ".npm-packages/bin",
                ".opencode/bin",
            ]
            .into_iter()
            .map(|dir| home.join(dir)),
        );
    }
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for name in names {
        for directory in &directories {
            for filename in executable_names(name) {
                let path = directory.join(filename);
                if !executable_file(&path) {
                    continue;
                }
                if let Ok(canonical) = fs::canonicalize(&path) {
                    if seen.insert(canonical) {
                        // Keep the invoked name: npm and multi-call executables
                        // can use argv[0] to select their command.
                        let path = if path.is_absolute() {
                            path
                        } else {
                            match env::current_dir() {
                                Ok(cwd) => cwd.join(path),
                                Err(_) => continue,
                            }
                        };
                        paths.push(path);
                    }
                }
            }
        }
    }
    paths
}

#[cfg(not(windows))]
fn executable_names(name: &str) -> Vec<String> {
    vec![name.to_owned()]
}

#[cfg(windows)]
fn executable_names(name: &str) -> Vec<String> {
    if Path::new(name).extension().is_some() {
        return vec![name.to_owned()];
    }
    let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    extensions
        .split(';')
        .filter(|ext| ext.starts_with('.') && ext.len() > 1)
        .map(|ext| format!("{name}{ext}"))
        .collect()
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn omp_overlay(db_path: &Path) -> Result<String> {
    let parent = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).context("Cannot create plugin data directory")?;
    let path = plain_path(fs::canonicalize(parent)?).join("omp-new.yml");
    safe_text(path_text(&path)?, "OMP overlay path")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(OMP_OVERLAY).and_then(|_| file.sync_all()) {
                let _ = fs::remove_file(&path);
                return Err(error).context("Cannot write fresh-chat OMP overlay");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != OMP_OVERLAY.len() as u64
                || fs::read(&path)? != OMP_OVERLAY
            {
                bail!("Generated OMP overlay {} is not the expected constant autoResume: false overlay; refusing to overwrite it", path.display());
            }
        }
        Err(error) => return Err(error).context("Cannot create fresh-chat OMP overlay"),
    }
    Ok(path_text(&path)?.to_owned())
}

#[cfg(not(windows))]
fn launch_command(argv: &[String], _home: &Path) -> Result<String> {
    Ok(crate::text::shell_join(argv))
}

#[cfg(windows)]
fn launch_command(argv: &[String], home: &Path) -> Result<String> {
    let encoded = powershell_encoded_script(argv)?;
    // Only fixed ASCII launcher names occur outside the encoded script. All
    // project-controlled values are PowerShell literals inside UTF-16LE Base64.
    let launcher = ["powershell.exe", "pwsh.exe"].into_iter().find(|name| {
        !candidates(home, &[*name], false).is_empty()
    }).context("Install PowerShell (powershell.exe or pwsh) and add it to PATH to launch chats safely on Windows")?;
    Ok(format!("{launcher} -NoProfile -EncodedCommand {encoded}"))
}

#[cfg(any(windows, test))]
fn powershell_encoded_script(argv: &[String]) -> Result<String> {
    use base64::Engine;
    let mut script = String::from("&");
    for arg in argv {
        safe_text(arg, "Launch argument")?;
        script.push_str(" '");
        script.push_str(&arg.replace('\'', "''"));
        script.push('\'');
    }
    script.push_str("; exit $LASTEXITCODE");
    let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Ok(base64::engine::general_purpose::STANDARD.encode(encoded))
}

fn probe(path: &Path, home: &Path, id: &str, args: &[&str]) -> Result<String> {
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Help must not accidentally consult a real user's store for fixture homes.
    // No model/provider/approval configuration is changed.
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0");
    match id {
        "codex" => {
            command
                .env("CODEX_HOME", home.join(".codex"))
                .env_remove("CODEX_REMOTE_URL");
        }
        "claude" => {
            command.env("CLAUDE_CONFIG_DIR", home.join(".claude"));
        }
        "omp" => {
            command.env("PI_CODING_AGENT_DIR", home.join(".omp/agent"));
        }
        "pi" => {
            command.env("PI_CODING_AGENT_DIR", home.join(".pi/agent"));
        }
        "opencode" => {
            command
                .env("XDG_DATA_HOME", home.join(".local/share"))
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("XDG_STATE_HOME", home.join(".local/state"))
                .env("XDG_CACHE_HOME", home.join(".cache"))
                .env("OPENCODE_CONFIG_DIR", home.join(".config/opencode"))
                .env_remove("OPENCODE_CONFIG")
                .env_remove("OPENCODE_CONFIG_CONTENT")
                .env_remove("OPENCODE_SERVER_URL")
                .env_remove("OPENCODE_SERVER_USERNAME")
                .env_remove("OPENCODE_SERVER_PASSWORD");
        }
        _ => {}
    }
    if home.is_dir() {
        command.current_dir(home);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot probe {}", path.display()))?;
    let stdout = child
        .stdout
        .take()
        .context("Help stdout pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("Help stderr pipe is unavailable")?;
    let output = bounded_reader(stdout);
    let errors = bounded_reader(stderr);
    let result = (|| -> Result<String> {
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                bail!("Help probe exceeded the 5 second timeout");
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = receive_output(output, deadline)?;
        let stderr = receive_output(errors, deadline)?;
        let stdout = String::from_utf8_lossy(&stdout);
        let stderr = String::from_utf8_lossy(&stderr);
        if !status.success() {
            bail!(
                "Help probe exited with {status}: {}",
                stderr.chars().take(500).collect::<String>().trim()
            );
        }
        let help = strip_ansi(&format!("{stdout}\n{stderr}"));
        if help.trim().is_empty() {
            bail!("Help probe returned no identifying output");
        }
        Ok(help)
    })();
    if result.is_err() {
        stop_probe(&mut child);
    }
    result.with_context(|| format!("{} {}", path.display(), args.join(" ")))
}

fn bounded_reader(reader: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take(MAX_PROBE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    receiver
}

fn receive_output(
    receiver: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    deadline: Instant,
) -> Result<Vec<u8>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let bytes = receiver
        .recv_timeout(remaining)
        .context("Help output exceeded the 5 second timeout")??;
    if bytes.len() as u64 > MAX_PROBE_BYTES {
        bail!("Help output exceeded the size limit");
    }
    Ok(bytes)
}

fn stop_probe(child: &mut Child) {
    #[cfg(unix)]
    {
        // The child leads its own process group; terminate npm/shell probe
        // descendants too, without touching the calling harness process group.
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn strip_ansi(input: &str) -> String {
    let mut clean = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.next() == Some('\\') {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            clean.push(ch);
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_harness_identity_from_incidental_brand_names() {
        assert!(identifies("cursor", "Cursor Agent\nUsage: agent [options]"));
        assert!(!identifies(
            "cursor",
            "Grok Build TUI\nUsage: agent [options]\nCursor Agent migration"
        ));
        assert!(!identifies(
            "codex",
            "A tool that can import Codex CLI logs"
        ));
        assert!(identifies("codex", "Codex CLI\nUsage: codex [OPTIONS]"));
        assert!(identifies("omp", "omp v18.11.0\nUSAGE"));
        assert!(!identifies("pi", "omp v18.11.0\nPI_CODING_AGENT_DIR"));
    }

    #[test]
    fn recognizes_declared_flags_only() {
        let help = "Use --add-dir for directories.\n  --add-dir-extra <path>\n  -C, --cd <DIR>\n  --config=<value>\n";
        assert!(!help_has_flag(help, "--add-dir"));
        assert!(help_has_flag(help, "--cd"));
        assert!(help_has_flag(help, "--config"));
        assert!(help_has_flag("  --add-dir <directories...>", "--add-dir"));
    }

    #[test]
    fn powershell_uses_utf16_literal_arguments_not_interpolated_commands() {
        use base64::Engine;
        let argv = vec![
            "C:\\Program Files\\Codex\\codex.exe".to_owned(),
            "--cd".to_owned(),
            "C:\\work\\it's $(Remove-Item *) & 中文🦀".to_owned(),
        ];
        let encoded = powershell_encoded_script(&argv).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(),
            "& 'C:\\Program Files\\Codex\\codex.exe' '--cd' 'C:\\work\\it''s $(Remove-Item *) & 中文🦀'; exit $LASTEXITCODE");
        assert!(powershell_encoded_script(&["bad\npath".to_owned()]).is_err());
        assert!(powershell_encoded_script(&["bad\0path".to_owned()]).is_err());
    }

    #[test]
    fn overlay_is_constant_and_never_overwrites_existing_content() {
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("index.sqlite");
        let path = PathBuf::from(omp_overlay(&db).unwrap());
        assert_eq!(fs::read(&path).unwrap(), OMP_OVERLAY);
        assert_eq!(omp_overlay(&db).unwrap(), path.to_str().unwrap());
        fs::write(&path, b"autoResume: true\n").unwrap();
        assert!(omp_overlay(&db).is_err());
        assert_eq!(fs::read(path).unwrap(), b"autoResume: true\n");
    }

    #[test]
    fn missing_primary_never_falls_back_to_home() {
        let data = tempfile::tempdir().unwrap();
        let error = launch(
            &data.path().join("index.sqlite"),
            data.path(),
            "omp",
            &[data
                .path()
                .join("missing-primary")
                .to_str()
                .unwrap()
                .to_owned()],
        )
        .unwrap_err();
        assert!(error.to_string().contains("missing-primary"));
    }
}
