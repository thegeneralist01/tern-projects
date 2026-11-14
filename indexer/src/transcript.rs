//! Read-only chat transcripts for the panel's preview pane.
//!
//! Sessions stream line by line and only the newest turns are retained, so a long chat never
//! loads into memory and one canvas update stays inside Tern's budget.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::text::clean_user;

/// Tern caps one canvas update at 256 KiB, measured on the encoded view; leave room for escaping.
const BUDGET: usize = 128 * 1024;
/// A pasted log or a long answer must not crowd out the rest of the chat.
const PART_CAP: usize = 16 * 1024;
const TURN_CAP: usize = 64 * 1024;
/// Tern caps a canvas view at 2000 nodes; each turn draws a few.
const MAX_TURNS: usize = 400;
/// One reply's blocks are buffered up to here before its oldest are dropped.
const TURN_BUFFER: usize = 2 * TURN_CAP;
const MAX_BLOCKS: usize = 512;
/// Condensed tool calls: clip each name, cap the line, and stop tracking new names past this.
const TOOL_NAME_CAP: usize = 64;
const TOOLS_CAP: usize = 4 * 1024;
const MAX_TOOL_NAMES: usize = 24;
/// A branching session keeps one map entry per entry; past this, treat the file as linear.
const MAX_TREE_ENTRIES: usize = 200_000;
/// Marks the front of a reply or chat that had to be cut to fit the budget.
const CUT: &str = "_… earlier part of this reply omitted_";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Role {
    User,
    Assistant,
}

/// `s` cut to at most `cap` bytes on a character boundary.
fn clip(s: &str, cap: usize) -> &str {
    if s.len() <= cap {
        return s;
    }
    let mut at = cap;
    while at > 0 && !s.is_char_boundary(at) {
        at -= 1;
    }
    &s[..at]
}

/// The last `cap` bytes of `s`, moved to a character boundary.
fn tail(s: &str, cap: usize) -> &str {
    if s.len() <= cap {
        return s;
    }
    let mut at = s.len() - cap;
    while at < s.len() && !s.is_char_boundary(at) {
        at += 1;
    }
    &s[at..]
}

/// Keep the end of `s`, marked as cut, within `cap` bytes.
fn cut_to_tail(s: &str, cap: usize) -> String {
    let rest = tail(s, cap.saturating_sub(CUT.len() + 2));
    let mut out = String::with_capacity(rest.len() + CUT.len() + 2);
    out.push_str(CUT);
    out.push_str("\n\n");
    out.push_str(rest);
    out
}

/// The marker and reopened fence a cut reply needs before its retained text.
fn markdown_head(cut: bool, fence: Option<(char, usize)>) -> String {
    let mut head = String::new();
    if cut {
        head.push_str(CUT);
        head.push_str("\n\n");
    }
    if let Some((marker, width)) = fence {
        head.push_str(&fence_line(marker, width));
        head.push('\n');
    }
    head
}

/// The first line start at or after `at`, moved to a character boundary.
fn line_start(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    match text[at..].find('\n') {
        Some(index) => at + index + 1,
        None => text.len(),
    }
}

/// The fence left open after reading `text`, continuing from `open`.
///
/// A closing fence matches the opening character and is at least as long as its opening run, so
/// ```` ``` ```` never closes a ```` ```` ```` fence that opened with four.
fn fence_after(open: Option<(char, usize)>, text: &str) -> Option<(char, usize)> {
    let mut open = open;
    for line in text.lines() {
        let line = line.trim_start();
        let Some(marker) = line.chars().next().filter(|c| *c == '`' || *c == '~') else {
            continue;
        };
        let run = line.len() - line.trim_start_matches(marker).len();
        if run < 3 {
            continue;
        }
        match open {
            None => open = Some((marker, run)),
            // Inside a fence only a same-character run of at least the opening length, with no
            // info string, closes it; everything shorter stays content.
            Some((current, width))
                if current == marker && run >= width && line[run..].trim().is_empty() =>
            {
                open = None
            }
            _ => {}
        }
    }
    open
}

/// A fence line that opens a block of the given character and width.
fn fence_line(marker: char, width: usize) -> String {
    std::iter::repeat_n(marker, width).collect()
}

/// Close a code fence left open, so one message cannot swallow the rest of the preview.
fn close_fences(text: &mut String) {
    if let Some((marker, width)) = fence_after(None, text) {
        text.push('\n');
        text.push_str(&fence_line(marker, width));
    }
}

/// One text block, capped and safe to render on its own.
fn text_block(text: &str) -> String {
    let truncated = text.len() > PART_CAP;
    // Control bytes (ANSI escapes in pasted logs) render as noise and inflate the encoded view.
    let mut out: String = clip(text, PART_CAP)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    close_fences(&mut out);
    if truncated {
        out.push_str("\n\n_… message truncated_");
    }
    out
}

/// Consecutive tool calls, condensed to one line with counts.
#[derive(Default)]
struct Tools {
    names: Vec<(String, usize)>,
    more: usize,
}

impl Tools {
    fn add(&mut self, name: &str) {
        let name = if name.len() > TOOL_NAME_CAP {
            format!("{}…", clip(name, TOOL_NAME_CAP))
        } else {
            name.to_owned()
        };
        if let Some(entry) = self.names.iter_mut().find(|(known, _)| *known == name) {
            entry.1 += 1;
        } else if self.names.len() < MAX_TOOL_NAMES {
            self.names.push((name, 1));
        } else {
            // A flood of distinct names must not grow the preview without bound.
            self.more += 1;
        }
    }

    fn line(&self) -> String {
        let mut out = String::from("_Tools: ");
        let mut shown = 0;
        for (name, count) in &self.names {
            let name = name.replace('`', "");
            let piece = if *count > 1 {
                format!("`{name}` ×{count}")
            } else {
                format!("`{name}`")
            };
            if shown > 0 {
                out.push_str(" · ");
            }
            out.push_str(&piece);
            shown += 1;
            if out.len() >= TOOLS_CAP.saturating_sub(32) {
                break;
            }
        }
        let left = self.names.len().saturating_sub(shown) + self.more;
        if left > 0 {
            out.push_str(&format!(" · …{left} more"));
        }
        out.push('_');
        if out.len() > TOOLS_CAP {
            let cut = clip(&out, TOOLS_CAP).len();
            out.truncate(cut);
        }
        out
    }
}

enum Block {
    Text(String),
    Tools(Tools),
}

fn block_text(block: &Block) -> String {
    match block {
        Block::Text(text) => text.clone(),
        Block::Tools(tools) => tools.line(),
    }
}

fn block_len(block: &Block) -> usize {
    match block {
        Block::Text(text) => text.len(),
        Block::Tools(tools) => tools.line().len(),
    }
}

/// One speaker's consecutive entries; oversized replies keep their end.
struct Turn {
    role: Role,
    blocks: VecDeque<Block>,
    used: usize,
    cut: bool,
    /// The fence open where the retained blocks begin, so a cut reply stays inside its code block.
    fence: Option<(char, usize)>,
}

impl Turn {
    fn new(role: Role) -> Self {
        Self { role, blocks: VecDeque::new(), used: 0, cut: false, fence: None }
    }

    fn trim(&mut self) {
        while self.blocks.len() > 1 && (self.used > TURN_BUFFER || self.blocks.len() > MAX_BLOCKS) {
            let Some(block) = self.blocks.pop_front() else { break };
            self.used = self.used.saturating_sub(block_len(&block) + 2);
            self.fence = fence_after(self.fence, &block_text(&block));
            self.cut = true;
        }
    }

    fn text(&mut self, text: String) {
        self.used += text.len() + 2;
        self.blocks.push_back(Block::Text(text));
        self.trim();
    }

    fn tool(&mut self, name: &str) {
        match self.blocks.back_mut() {
            Some(Block::Tools(tools)) => {
                let before = tools.line().len();
                tools.add(name);
                self.used = self.used.saturating_sub(before).saturating_add(tools.line().len());
            }
            _ => {
                let mut tools = Tools::default();
                tools.add(name);
                self.used += tools.line().len() + 2;
                self.blocks.push_back(Block::Tools(tools));
            }
        }
        self.trim();
    }

    /// Markdown for this turn, never longer than [`TURN_CAP`].
    ///
    /// A cut reply keeps its end; when that end starts inside a fenced block whose opening was
    /// dropped, a matching fence is put back so the surviving closing fence still closes it.
    fn markdown(&self) -> String {
        let joined = self.blocks.iter().map(block_text).collect::<Vec<String>>().join("\n\n");
        let mut cut = self.cut;
        let mut start = 0;
        let mut fence = self.fence;
        for _ in 0..3 {
            let head = markdown_head(cut, fence);
            let budget = TURN_CAP.saturating_sub(head.len());
            if joined.len() - start <= budget {
                return format!("{head}{}", &joined[start..]);
            }
            cut = true;
            let at = line_start(&joined, joined.len() - budget);
            if at <= start {
                break;
            }
            fence = fence_after(self.fence, &joined[..at]);
            start = at;
        }
        // Pathological fences keep changing the head: keep the last measured tail.
        let head = markdown_head(cut, fence);
        let at = line_start(&joined, joined.len().saturating_sub(TURN_CAP.saturating_sub(head.len())));
        format!("{head}{}", &joined[at.min(joined.len())..])
    }
}

/// The newest turns that fit the budget, oldest first, and how many turns were skipped.
#[derive(Default)]
struct Preview {
    turns: VecDeque<(Role, String)>,
    used: usize,
    total: usize,
}

impl Preview {
    fn push(&mut self, role: Role, markdown: String) {
        self.total += 1;
        let markdown = if markdown.len() > BUDGET { cut_to_tail(&markdown, BUDGET) } else { markdown };
        self.used += markdown.len() + 2;
        self.turns.push_back((role, markdown));
        while self.turns.len() > MAX_TURNS || (self.used > BUDGET && self.turns.len() > 1) {
            let Some((_, old)) = self.turns.pop_front() else { break };
            self.used = self.used.saturating_sub(old.len() + 2);
        }
        // The newest turn alone must also fit, so no single block can overflow the update.
        if self.used > BUDGET {
            if let Some((_, newest)) = self.turns.front_mut() {
                *newest = cut_to_tail(newest, BUDGET);
            }
            self.used = self.turns.iter().map(|(_, markdown)| markdown.len() + 2).sum();
        }
    }

    fn hidden(&self) -> usize {
        self.total.saturating_sub(self.turns.len())
    }
}

/// Turns grouped as they stream in; the preview keeps only its bounded tail.
struct Builder {
    preview: Preview,
    current: Option<Turn>,
}

impl Builder {
    fn new() -> Self {
        Self { preview: Preview::default(), current: None }
    }

    fn flush(&mut self) {
        if let Some(turn) = self.current.take() {
            self.preview.push(turn.role, turn.markdown());
        }
    }

    fn ensure(&mut self, role: Role) {
        if !matches!(&self.current, Some(turn) if turn.role == role) {
            self.flush();
            self.current = Some(Turn::new(role));
        }
    }

    fn say(&mut self, role: Role, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.ensure(role);
        let block = text_block(text);
        if let Some(turn) = self.current.as_mut() {
            turn.text(block);
        }
    }

    fn tool(&mut self, name: &str) {
        self.ensure(Role::Assistant);
        if let Some(turn) = self.current.as_mut() {
            turn.tool(name);
        }
    }

    fn user_text(&mut self, text: &str) {
        if let Some(text) = clean_user(text) {
            self.say(Role::User, &text);
        }
    }

    /// A user record: text blocks stay, tool output (and screenshots returned with it) does not.
    fn user(&mut self, content: Option<&Value>) {
        let mut text = String::new();
        let mut images = 0;
        match content {
            Some(Value::String(s)) => text.push_str(s),
            Some(Value::Array(items)) => {
                let tool_output = items
                    .iter()
                    .any(|item| item.get("type").and_then(Value::as_str) == Some("tool_result"));
                for item in items {
                    match item.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = item.get("text").and_then(Value::as_str) {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                            }
                        }
                        Some("image") if !tool_output => images += 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let mut text = clean_user(&text).unwrap_or_default();
        if images > 0 {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&format!("_[{images} image{}]_", if images == 1 { "" } else { "s" }));
        }
        if !text.is_empty() {
            self.say(Role::User, &text);
        }
    }

    fn assistant(&mut self, content: Option<&Value>, tool_type: &str) {
        match content {
            Some(Value::String(s)) => self.say(Role::Assistant, s),
            Some(Value::Array(items)) => {
                for item in items {
                    match item.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = item.get("text").and_then(Value::as_str) {
                                self.say(Role::Assistant, t);
                            }
                        }
                        Some(kind) if kind == tool_type => {
                            self.tool(item.get("name").and_then(Value::as_str).unwrap_or("tool"));
                        }
                        // Thinking stays out of the preview.
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Preview {
        self.flush();
        self.preview
    }
}

/// Run `each` over the file's lines without loading it whole, reading at most `limit` bytes.
fn each_line(path: &Path, limit: u64, mut each: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let file = File::open(path).with_context(|| format!("Reading {}", path.display()))?;
    let mut reader = BufReader::new(file.take(limit));
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        each(line.trim_end_matches(['\n', '\r']))?;
    }
}

/// The file's length and modification time, used to pin one consistent snapshot of a session.
fn file_stamp(path: &Path) -> Result<(u64, Option<std::time::SystemTime>)> {
    let meta = std::fs::metadata(path).with_context(|| format!("Reading {}", path.display()))?;
    Ok((meta.len(), meta.modified().ok()))
}

/// Run `read` against one snapshot of `path`, retrying both passes once if the file is rewritten.
///
/// A longer file is an append to the same prefix, so it stays readable; a shorter file or a
/// rewrite at the same length would mix two versions and is retried, then refused.
fn stable_snapshot<T>(path: &Path, mut read: impl FnMut(u64) -> Result<T>) -> Result<T> {
    for attempt in 0..2 {
        let (len, modified) = file_stamp(path)?;
        let value = read(len)?;
        let (now_len, now_modified) = file_stamp(path)?;
        if now_len > len || (now_len == len && now_modified == modified) {
            return Ok(value);
        }
        if attempt == 1 {
            bail!("This chat changed while loading. Try again.");
        }
    }
    unreachable!("the retry above either returns or fails")
}

/// The entries from the file's last entry back to the root.
fn active_branch(parents: &HashMap<String, Option<String>>, tip: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut at = Some(tip);
    while let Some(id) = at {
        if !ids.insert(id.to_owned()) {
            break;
        }
        at = parents.get(id).and_then(|parent| parent.as_deref());
    }
    ids
}

/// OMP sessions are trees: `/tree` and rewinds leave abandoned entries in the file, so only the
/// branch that ends at the last chat message is the conversation. Two streaming passes over one
/// snapshot keep one small id map instead of every parsed entry.
fn read_omp(path: &Path) -> Result<Preview> {
    stable_snapshot(path, |len| omp_snapshot(path, len))
}

/// One bounded read of an OMP session: the branch is chosen in the first pass and rendered in the
/// second, both over the same `limit` bytes.
fn omp_snapshot(path: &Path, limit: u64) -> Result<Preview> {
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    let mut tip: Option<String> = None;
    let mut linked = false;
    let mut overflow = false;
    each_line(path, limit, |line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else { return Ok(()) };
        let Some(id) = value.get("id").and_then(Value::as_str) else { return Ok(()) };
        let is_message = value.get("type").and_then(Value::as_str) == Some("message");
        let parent = value.get("parentId");
        // Only chat messages name the live branch: bookkeeping records (model usage, title
        // changes) are appended after them, and one on an older branch must not become the tip.
        if is_message {
            linked = linked || parent.is_some();
            if parent.is_some() || tip.is_none() {
                tip = Some(id.to_owned());
            }
        }
        // Metadata records (the session header, title changes) carry ids but not the tree.
        if parent.is_none() && !is_message {
            return Ok(());
        }
        if parents.len() >= MAX_TREE_ENTRIES {
            overflow = true;
            return Ok(());
        }
        parents.insert(id.to_owned(), parent.and_then(Value::as_str).map(str::to_owned));
        Ok(())
    })?;
    // Sessions without parent links, or too long to map, render as written rather than as one branch.
    let branch = match tip {
        Some(tip) if linked && !overflow => Some(active_branch(&parents, &tip)),
        _ => None,
    };
    let mut out = Builder::new();
    each_line(path, limit, |line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else { return Ok(()) };
        if let Some(branch) = &branch {
            let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
            if !branch.contains(id) {
                return Ok(());
            }
        }
        if value.get("type").and_then(Value::as_str) != Some("message") {
            return Ok(());
        }
        let msg = value.get("message").unwrap_or(&value);
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => out.user(msg.get("content")),
            Some("assistant") => out.assistant(msg.get("content"), "toolCall"),
            _ => {}
        }
        Ok(())
    })?;
    Ok(out.finish())
}

/// Claude Code writes one JSONL record per step; summaries, sidechains and machine-origin records
/// are not the conversation. Queued prompts reach the agent as attachment records.
fn read_claude(path: &Path) -> Result<Preview> {
    let mut out = Builder::new();
    each_line(path, u64::MAX, |line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else { return Ok(()) };
        let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
        // Compaction summaries are written as user records but are Claude Code's own text.
        if flag("isSidechain") || flag("isMeta") || flag("isCompactSummary") {
            return Ok(());
        }
        // Humans own a record's text; task notifications and agent origins do not.
        let machine = value
            .pointer("/origin/kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "human");
        if machine {
            return Ok(());
        }
        let content = value.pointer("/message/content").or_else(|| value.get("content"));
        match value.get("type").and_then(Value::as_str) {
            Some("user") => out.user(content),
            Some("assistant") => out.assistant(content, "tool_use"),
            Some("attachment") => {
                let attachment = &value["attachment"];
                // `commandMode` separates prompts the user queued from background notifications.
                if attachment.get("type").and_then(Value::as_str) == Some("queued_command")
                    && attachment.get("commandMode").and_then(Value::as_str) == Some("prompt")
                {
                    if let Some(prompt) = attachment.get("prompt").and_then(Value::as_str) {
                        out.user_text(prompt);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(out.finish())
}

/// Only a real file inside the harness's own history folder is ever opened.
fn transcript_path(root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    crate::delete::safe_path(root, path, false)
        .with_context(|| format!("Opening the {label} transcript"))
}

pub fn preview(db_path: &Path, home: &Path, uid: &str) -> Result<Value> {
    let conn = crate::db::open_readonly(db_path)?;
    let (agent, source, title, cwd): (String, String, String, String) = conn
        .query_row("SELECT agent, source, title, cwd FROM sessions WHERE uid=?1", [uid], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?
        .context("Chat not found. Refresh the index and try again.")?;
    let (label, root) = match agent.as_str() {
        "omp" => ("OMP", home.join(".omp/agent/sessions")),
        "claude" => ("Claude Code", home.join(".claude/projects")),
        other => bail!("Preview is not available for {other} chats."),
    };
    let path = Path::new(source.strip_prefix(&format!("{agent}:")).context("Unexpected chat source")?);
    let path = transcript_path(&root, path, label)?;
    let transcript = if agent == "omp" { read_omp(&path)? } else { read_claude(&path)? };
    let hidden = transcript.hidden();
    let turns: Vec<Value> = transcript
        .turns
        .into_iter()
        .map(|(role, markdown)| {
            json!({"role": if role == Role::User { "user" } else { "assistant" }, "markdown": markdown})
        })
        .collect();
    Ok(json!({
        "uid": uid,
        "title": title,
        "agent": agent,
        "label": label,
        "cwd": cwd,
        "hidden": hidden,
        "turns": turns,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temporary directory removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let seq = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "tern-projects-transcript-{}-{name}-{seq}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, content: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn claude(lines: &[&str]) -> Preview {
        let scratch = Scratch::new("claude");
        read_claude(&scratch.file("chat.jsonl", &lines.join("\n"))).unwrap()
    }

    fn rendered(preview: &Preview) -> Vec<(Role, String)> {
        preview.turns.iter().cloned().collect()
    }

    #[test]
    fn tool_calls_between_replies_condense_into_one_line() {
        let jsonl = [
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"Fix it"}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret"},{"type":"text","text":"Looking."},{"type":"toolCall","name":"read"}]}}"#,
            r#"{"type":"message","message":{"role":"toolResult","content":[{"type":"text","text":"file body"}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"read"},{"type":"toolCall","name":"edit"}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}"#,
        ];
        // The tree is the same file with one parent chain; ids tie the entries together.
        let scratch = Scratch::new("omp-tools");
        let mut lines = vec![r#"{"type":"title","v":1,"title":""}"#.to_owned()];
        let mut parent = "null".to_owned();
        for (i, line) in jsonl.iter().enumerate() {
            let id = format!("i{i}");
            let mut value: Value = serde_json::from_str(line).unwrap();
            value["id"] = json!(id);
            value["parentId"] = serde_json::from_str(&parent).unwrap();
            lines.push(value.to_string());
            parent = format!("\"{id}\"");
        }
        let preview = read_omp(&scratch.file("chat.jsonl", &lines.join("\n"))).unwrap();
        assert_eq!(preview.hidden(), 0);
        assert_eq!(rendered(&preview), vec![
            (Role::User, "Fix it".to_owned()),
            (Role::Assistant, "Looking.\n\n_Tools: `read` ×2 · `edit`_\n\nDone.".to_owned()),
        ]);
    }

    #[test]
    fn long_chats_keep_the_newest_turns_within_budget() {
        let body = "x".repeat(PART_CAP);
        let lines: Vec<String> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                json!({"type": role, "message": {"content": [{"type": "text", "text": format!("turn{i} {body}")}]}})
                    .to_string()
            })
            .collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let preview = claude(&refs);
        let turns = rendered(&preview);
        let size: usize = turns.iter().map(|(_, md)| md.len()).sum();
        assert!(size <= BUDGET, "{size}");
        assert!(preview.hidden() > 0 && preview.hidden() + turns.len() == 40);
        assert!(turns.last().is_some_and(|(_, md)| md.starts_with("turn39 ")), "newest turn kept");
        assert!(turns.iter().all(|(_, md)| !md.starts_with("turn0 ")), "oldest turn dropped");
    }

    #[test]
    fn claude_summaries_and_tool_output_never_appear_as_user_turns() {
        let jsonl = [
            r#"{"type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued from a previous conversation"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"Pick a tree"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"AskUserQuestion","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"chosen"},{"type":"image","source":{}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Using tree 3."}]}}"#,
        ];
        let preview = claude(&jsonl);
        assert_eq!(preview.hidden(), 0);
        assert_eq!(rendered(&preview), vec![
            (Role::User, "Pick a tree".to_owned()),
            (Role::Assistant, "_Tools: `AskUserQuestion`_\n\nUsing tree 3.".to_owned()),
        ]);
    }

    #[test]
    fn claude_keeps_user_text_beside_tool_output_and_shows_queued_prompts() {
        let jsonl = [
            // The user's own words survive a record that also carries tool output.
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"file"},{"type":"text","text":"and also this"}]}}"#,
            // A prompt queued while the agent worked is a user turn.
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"queued question","commandMode":"prompt"}}"#,
            // A human-origin queued prompt is still a user turn.
            r#"{"type":"attachment","origin":{"kind":"human"},"attachment":{"type":"queued_command","prompt":"typed later","commandMode":"prompt"}}"#,
            // Task notifications are the machine talking, whichever record shape carries them.
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"background finished","commandMode":"task-notification"}}"#,
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>done</task-notification>"}}"#,
            // A non-human origin never becomes a user turn.
            r#"{"type":"attachment","origin":{"kind":"agent"},"attachment":{"type":"queued_command","prompt":"from another agent","commandMode":"prompt"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"On it."}]}}"#,
        ];
        let preview = claude(&jsonl);
        assert_eq!(preview.hidden(), 0);
        // Consecutive user entries form one turn; each source contributed its own block.
        assert_eq!(rendered(&preview), vec![
            (Role::User, "and also this\n\nqueued question\n\ntyped later".to_owned()),
            (Role::Assistant, "On it.".to_owned()),
        ]);
    }

    #[test]
    fn omp_preview_follows_only_the_active_branch() {
        let jsonl = [
            // The first message may be the root; metadata records close the file.
            r#"{"type":"message","id":"a","message":{"role":"user","content":[{"type":"text","text":"root"}]}}"#,
            r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"first answer"}]}}"#,
            // Rewound away: never on the path that ends at the last entry.
            r#"{"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"abandoned answer"}]}}"#,
            r#"{"type":"message","id":"d","parentId":"c","message":{"role":"assistant","content":[{"type":"text","text":"abandoned follow-up"}]}}"#,
            r#"{"type":"message","id":"e","parentId":"b","message":{"role":"user","content":[{"type":"text","text":"follow-up"}]}}"#,
            r#"{"type":"message","id":"f","parentId":"e","message":{"role":"assistant","content":[{"type":"text","text":"second answer"}]}}"#,
            r#"{"type":"title_change","id":"relay-rename","title":"renamed"}"#,
        ];
        let preview = claude_omp(&jsonl);
        assert_eq!(rendered(&preview), vec![
            (Role::User, "root".to_owned()),
            (Role::Assistant, "first answer".to_owned()),
            (Role::User, "follow-up".to_owned()),
            (Role::Assistant, "second answer".to_owned()),
        ]);
    }

    #[test]
    fn omp_sessions_without_parent_links_render_every_message() {
        // The checked-in OMP fixtures carry message ids but no `parentId`.
        let jsonl = [
            r#"{"type":"session","id":"session-omp1","cwd":"/project","timestamp":1717236000000}"#,
            r#"{"type":"title_change","title":"OMP indexer session","timestamp":1717236001000}"#,
            r#"{"type":"message","id":"msg1","timestamp":1717236003000,"message":{"role":"user","content":[{"type":"text","text":"one"}]}}"#,
            r#"{"type":"message","id":"msg2","timestamp":1717236004000,"message":{"role":"assistant","content":[{"type":"text","text":"two"}]}}"#,
            r#"{"type":"message","id":"msg3","timestamp":1717236005000,"message":{"role":"user","content":[{"type":"text","text":"three"}]}}"#,
        ];
        let preview = claude_omp(&jsonl);
        assert_eq!(rendered(&preview), vec![
            (Role::User, "one".to_owned()),
            (Role::Assistant, "two".to_owned()),
            (Role::User, "three".to_owned()),
        ]);
    }

    #[test]
    fn omp_bookkeeping_entries_never_pick_the_branch() {
        let jsonl = [
            r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"root"}]}}"#,
            r#"{"type":"message","id":"c","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"abandoned"}]}}"#,
            r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"live"}]}}"#,
            // Bookkeeping on the older branch is written last; it is not a chat message.
            r#"{"type":"model_usage","id":"u1","parentId":"c","tokens":42}"#,
        ];
        let preview = claude_omp(&jsonl);
        assert_eq!(rendered(&preview), vec![
            (Role::User, "root".to_owned()),
            (Role::Assistant, "live".to_owned()),
        ]);
    }

    #[test]
    fn omp_snapshot_retries_a_rewritten_session_and_then_fails() {
        let scratch = Scratch::new("rewrite");
        let path = scratch.file("chat.jsonl", "12345");
        let mut reads = 0;
        let result: Result<u64> = stable_snapshot(&path, |len| {
            reads += 1;
            // The session is rewritten shorter during each read; both passes must retry, then
            // refuse. Shrinking every time keeps this independent of file-time resolution.
            std::fs::write(&path, if reads == 1 { "12" } else { "1" }).unwrap();
            Ok(len)
        });
        assert_eq!(reads, 2, "retry both passes once, then refuse");
        assert_eq!(result.unwrap_err().to_string(), "This chat changed while loading. Try again.");
    }

    #[test]
    fn omp_snapshot_ignores_messages_appended_while_loading() {
        let jsonl = [
            r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"one"}]}}"#,
            r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"text","text":"two"}]}}"#,
        ];
        let scratch = Scratch::new("append");
        let path = scratch.file("chat.jsonl", &format!("{}\n", jsonl.join("\n")));
        let snapshot = std::fs::metadata(&path).unwrap().len();
        // An append after the snapshot falls outside both passes instead of only one of them.
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, r#"{{"type":"message","id":"c","parentId":"b","message":{{"role":"user","content":[{{"type":"text","text":"three"}}]}}}}"#).unwrap();
        }
        let preview = omp_snapshot(&path, snapshot).unwrap();
        assert_eq!(rendered(&preview), vec![
            (Role::User, "one".to_owned()),
            (Role::Assistant, "two".to_owned()),
        ]);
        // Without the snapshot bound the appended message would appear in the second pass.
        let full = rendered(&read_omp(&path).unwrap());
        assert_eq!(full.len(), 3, "{full:?}");
    }

    #[test]
    fn cut_reply_restores_the_fence_it_starts_inside() {
        let mut builder = Builder::new();
        // The turn cap cuts through this fenced reply: its closing fence survives, its opener does not.
        builder.say(Role::Assistant, &format!("```\n{}\n```", "c\n".repeat(7_000)));
        for _ in 0..3 {
            builder.say(Role::Assistant, &"p".repeat(16_000));
        }
        builder.say(Role::Assistant, &"q".repeat(9_342));
        let preview = builder.finish();
        let markdown = &preview.turns.front().expect("one turn").1;
        assert!(
            markdown.starts_with(&format!("{CUT}\n\n```\n")),
            "a matching opener precedes the tail: {:?}",
            &markdown[..markdown.len().min(80)]
        );
        assert!(markdown.len() <= TURN_CAP, "{}", markdown.len());
        let fences = markdown.lines().filter(|line| line.trim_start().starts_with("```")).count();
        assert_eq!(fences % 2, 0, "the surviving closing fence still closes");
    }

    #[test]
    fn truncation_closes_the_longest_open_fence() {
        let body = "let a = 1;\n".repeat(PART_CAP);
        // Four backticks open; a three-backtick line inside is content, not a close.
        let out = text_block(&format!("````rust\n```\n{body}"));
        assert!(out.starts_with("````rust\n```\n"), "inner run stays content");
        assert!(out.contains("_… message truncated_"));
        assert!(
            out.ends_with("\n````\n\n_… message truncated_"),
            "closing fence must match the opening run: {:?}",
            &out[out.len().saturating_sub(60)..]
        );
        // A longer opening run is never closed by a shorter one.
        let out = text_block(&format!("`````\n````\n{body}"));
        assert!(out.ends_with("\n`````\n\n_… message truncated_"));
        // Tilde fences close with tildes of at least the opening length.
        let out = text_block(&format!("~~~~\n~~~\n{body}"));
        assert!(out.ends_with("\n~~~~\n\n_… message truncated_"));
    }

    #[test]
    fn tool_summaries_and_oversized_blocks_stay_within_the_budget() {
        let mut builder = Builder::new();
        builder.tool(&"n".repeat(TOOL_NAME_CAP * 4));
        // Long, distinct names would grow an unbounded summary well past its cap.
        for i in 0..80 {
            builder.tool(&format!("{i}{}", "m".repeat(200)));
        }
        builder.say(Role::Assistant, &"y".repeat(4 * 1024 * 1024));
        builder.say(Role::User, "after");
        let preview = builder.finish();
        let size: usize = preview.turns.iter().map(|(_, md)| md.len()).sum();
        assert!(size <= BUDGET, "{size} bytes exceed the budget");
        assert!(preview.turns.iter().all(|(_, md)| md.len() <= TURN_CAP));
        let tool_line = preview
            .turns
            .iter()
            .filter_map(|(_, md)| md.lines().find(|line| line.starts_with("_Tools:")))
            .max_by_key(|line| line.len())
            .expect("tool line");
        assert!(tool_line.len() <= TOOLS_CAP, "{} bytes", tool_line.len());
        assert!(tool_line.contains("…"), "long names are clipped");
        assert!(tool_line.contains("more"), "omitted names and calls stay counted");
        assert!(tool_line.split(" · ").count() <= MAX_TOOL_NAMES + 1);
    }

    #[test]
    fn streaming_retains_only_the_newest_turns() {
        let mut builder = Builder::new();
        for i in 0..2_000 {
            builder.say(Role::User, &format!("question {i}"));
            builder.say(Role::Assistant, &format!("answer {i}"));
            // Retained state stays bounded while the file is still streaming.
            assert!(builder.preview.turns.len() <= MAX_TURNS);
            assert!(builder.preview.used <= BUDGET + 2 * MAX_TURNS);
            if let Some(turn) = builder.current.as_ref() {
                assert!(turn.used <= TURN_BUFFER + PART_CAP);
            }
        }
        let preview = builder.finish();
        assert_eq!(preview.total, 4_000);
        assert_eq!(preview.turns.len(), MAX_TURNS);
        assert_eq!(preview.hidden(), 4_000 - MAX_TURNS);
        assert!(preview.turns.back().is_some_and(|(_, md)| md.starts_with("answer 1999")));
    }

    #[test]
    fn oversized_single_line_output_stays_within_the_budget() {
        // One enormous JSONL line must not push the preview past the budget.
        let line = json!({
            "type": "user",
            "message": {"content": [{"type": "text", "text": "z".repeat(8 * 1024 * 1024)}]},
        });
        let preview = claude(&[&line.to_string()]);
        let size: usize = preview.turns.iter().map(|(_, md)| md.len()).sum();
        assert!(size <= BUDGET, "{size}");
        assert!(preview.turns.front().is_some_and(|(_, md)| md.len() <= TURN_CAP));
    }

    #[cfg(unix)]
    #[test]
    fn transcripts_through_symlinks_or_outside_the_store_are_refused() {
        let scratch = Scratch::new("symlink");
        let root = scratch.0.join("sessions");
        std::fs::create_dir_all(&root).unwrap();
        let outside = scratch.file("outside.jsonl", "{}");
        let real = root.join("real.jsonl");
        std::fs::write(&real, "{}").unwrap();
        assert!(transcript_path(&root, &real, "OMP").is_ok());

        // A symlinked transcript, and a transcript under a symlinked folder, both escape the store.
        let file_link = root.join("linked.jsonl");
        std::os::unix::fs::symlink(&outside, &file_link).unwrap();
        assert!(transcript_path(&root, &file_link, "OMP").is_err());
        let dir = scratch.0.join("elsewhere");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chat.jsonl"), "{}").unwrap();
        let dir_link = root.join("linked-dir");
        std::os::unix::fs::symlink(&dir, &dir_link).unwrap();
        assert!(transcript_path(&root, &dir_link.join("chat.jsonl"), "OMP").is_err());

        // Plain lexical escapes are refused too.
        assert!(transcript_path(&root, &outside, "OMP").is_err());
        assert!(transcript_path(&root, &root.join("..").join("outside.jsonl"), "OMP").is_err());
        assert!(transcript_path(&root, &root.join("missing.jsonl"), "OMP").is_err());
    }

    fn claude_omp(lines: &[&str]) -> Preview {
        let scratch = Scratch::new("omp-branch");
        read_omp(&scratch.file("chat.jsonl", &lines.join("\n"))).unwrap()
    }
}
