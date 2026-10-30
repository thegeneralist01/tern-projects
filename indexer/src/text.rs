use std::path::PathBuf;

/// Clean user-facing text: remove system-reminder blocks and drop records that
/// are internal tooling noise. Returns None when the result is empty or noisy.
pub fn clean_user(s: &str) -> Option<String> {
    // Strip <system-reminder>…</system-reminder> blocks.
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<system-reminder>") {
        result.push_str(&rest[..start]);
        if let Some(end) = rest.find("</system-reminder>") {
            rest = &rest[end + "</system-reminder>".len()..];
        } else {
            // Unclosed tag — drop the rest.
            rest = "";
            break;
        }
    }
    result.push_str(rest);

    let trimmed = result.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Drop records that are internal CLI/tool noise.
    let noise_prefixes = [
        "<command-name>",
        "<command-message>",
        "<local-command-stdout>",
        "<local-command-caveat>",
        "<environment_context>",
        "<user_instructions>",
        "# AGENTS.md instructions",
        "<task-resumption-reminder>",
        "<context>",
    ];
    for pfx in &noise_prefixes {
        if trimmed.starts_with(pfx) {
            return None;
        }
    }

    Some(trimmed.to_string())
}

/// Collapse all whitespace runs to a single space, trim, and truncate with `…` at `max_chars`
/// codepoints.
pub fn one_line(s: &str, max_chars: usize) -> String {
    let collapsed: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let collapsed = collapsed.trim();
    if collapsed.chars().count() <= max_chars {
        collapsed.to_string()
    } else {
        let truncated: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
        truncated + "…"
    }
}

/// Truncate `s` to `max_bytes` on a char boundary, appending nothing (used for body text).
pub fn cap_body(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    // Walk back from max_bytes to a char boundary.
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    s.truncate(boundary);
}

/// Shell-quote an argument list: single-quote args containing any character
/// outside [A-Za-z0-9_@%+=:,./-], escaping embedded single-quotes as '\''.
pub fn shell_join(args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        if arg.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-')
        }) {
            parts.push(arg.clone());
        } else {
            // Single-quote the whole arg, escaping internal single-quotes.
            let escaped = arg.replace('\'', "'\\''");
            parts.push(format!("'{escaped}'"));
        }
    }
    parts.join(" ")
}

/// Parse an RFC 3339 timestamp (UTC Z or ±HH:MM offset) to milliseconds since epoch.
/// Handles `YYYY-MM-DDTHH:MM:SS[.fff]Z` and `YYYY-MM-DDTHH:MM:SS[.fff]±HH:MM`.
pub fn iso_ms(s: &str) -> Option<i64> {
    // Minimum: YYYY-MM-DDTHH:MM:SSZ (20 chars)
    let s = s.trim();
    if s.len() < 20 {
        return None;
    }

    let (date_time, offset_str) = if let Some(z) = s.strip_suffix('Z') {
        (z, "+00:00")
    } else if s.len() >= 25 {
        let (dt, off) = s.split_at(s.len() - 6);
        if off.starts_with('+') || off.starts_with('-') {
            (dt, off)
        } else {
            return None;
        }
    } else {
        return None;
    };

    let (date_part, time_part) = date_time.split_once('T')?;
    let mut dp = date_part.splitn(3, '-');
    let year: i64 = dp.next()?.parse().ok()?;
    let month: i64 = dp.next()?.parse().ok()?;
    let day: i64 = dp.next()?.parse().ok()?;

    // Time may have fractional seconds.
    let (hms, frac_str) = time_part.split_once('.').unwrap_or((time_part, ""));
    let mut tp = hms.splitn(3, ':');
    let hour: i64 = tp.next()?.parse().ok()?;
    let min: i64 = tp.next()?.parse().ok()?;
    let sec: i64 = tp.next()?.parse().ok()?;

    // Fractional seconds (up to 3 digits for ms).
    let ms_frac: i64 = if frac_str.is_empty() {
        0
    } else {
        let digits: String = frac_str.chars().take(3).collect();
        let n: i64 = digits.parse().ok()?;
        // Pad to 3 digits.
        let pad = 3 - digits.len() as i64;
        n * 10_i64.pow(pad as u32)
    };

    // Offset in minutes.
    let offset_min: i64 = if offset_str == "+00:00" || offset_str == "-00:00" {
        0
    } else {
        let sign: i64 = if offset_str.starts_with('-') { -1 } else { 1 };
        let rest = &offset_str[1..];
        let (oh, om) = rest.split_once(':')?;
        let oh: i64 = oh.parse().ok()?;
        let om: i64 = om.parse().ok()?;
        sign * (oh * 60 + om)
    };

    // Days from epoch (1970-01-01) using proleptic Gregorian calendar.
    let epoch_days = days_from_epoch(year, month, day)?;

    let utc_sec = epoch_days * 86400 + hour * 3600 + min * 60 + sec - offset_min * 60;
    Some(utc_sec * 1000 + ms_frac)
}

fn days_from_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    if month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    // Shift so that March is month 0 (simplifies leap year handling).
    let (y, m) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * m + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468; // days since 1970-01-01
    Some(days)
}

/// Undo the Windows verbatim prefix `std::fs::canonicalize` adds (`\\?\C:\...`,
/// `\\?\UNC\server\share`). Harnesses, shells and the project library treat `?`
/// as a literal character, so a verbatim spelling breaks per-project state
/// (OMP keys its session directory on the cwd and fails with ENOENT). Keep the
/// canonical spelling, drop the prefix.
pub fn plain_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_join_plain() {
        let args: Vec<String> = vec!["omp".into(), "--resume".into(), "/home/user/file.jsonl".into()];
        assert_eq!(shell_join(&args), "omp --resume /home/user/file.jsonl");
    }

    #[test]
    fn shell_join_space() {
        let args: Vec<String> = vec!["omp".into(), "--resume".into(), "/home/user/my sessions/file.jsonl".into()];
        assert_eq!(shell_join(&args), "omp --resume '/home/user/my sessions/file.jsonl'");
    }

    #[test]
    fn shell_join_single_quote() {
        let args: Vec<String> = vec!["echo".into(), "it's".into()];
        assert_eq!(shell_join(&args), "echo 'it'\\''s'");
    }

    #[test]
    fn iso_ms_utc() {
        // 2024-01-15T12:00:00Z
        let ms = iso_ms("2024-01-15T12:00:00Z").unwrap();
        // Verify it's a reasonable timestamp (2024 > 2020 and < 2030).
        assert!(ms > 1_700_000_000_000);
        assert!(ms < 1_900_000_000_000);
    }

    #[test]
    fn iso_ms_frac() {
        let a = iso_ms("2024-01-15T12:00:00.000Z").unwrap();
        let b = iso_ms("2024-01-15T12:00:00.123Z").unwrap();
        assert_eq!(b - a, 123);
    }

    #[test]
    fn iso_ms_offset() {
        let utc = iso_ms("2024-01-15T12:00:00Z").unwrap();
        let plus1 = iso_ms("2024-01-15T13:00:00+01:00").unwrap();
        assert_eq!(utc, plus1);
    }

    #[test]
    fn clean_user_strips_reminder() {
        let s = "hello<system-reminder>secret</system-reminder>world";
        assert_eq!(clean_user(s).unwrap(), "helloworld");
    }

    #[test]
    fn clean_user_noise() {
        assert!(clean_user("<command-name>foo</command-name>").is_none());
        assert!(clean_user("<environment_context>stuff</environment_context>").is_none());
    }

    #[test]
    fn one_line_truncates() {
        let s = "hello world this is a long sentence";
        let r = one_line(s, 10);
        assert!(r.len() <= 12); // 9 chars + ellipsis (3 bytes)
        assert!(r.ends_with('…'));
    }

    #[test]
    fn one_line_collapses() {
        assert_eq!(one_line("  foo   bar  ", 100), "foo bar");
    }

    #[cfg(windows)]
    #[test]
    fn plain_path_strips_verbatim_prefix() {
        assert_eq!(
            plain_path(PathBuf::from(r"\\?\D:\Users\charm\x")),
            PathBuf::from(r"D:\Users\charm\x")
        );
        assert_eq!(
            plain_path(PathBuf::from(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
        assert_eq!(plain_path(PathBuf::from(r"D:\already\plain")), PathBuf::from(r"D:\already\plain"));
    }
}
