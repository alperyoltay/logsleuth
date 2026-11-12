//! Line-level parsing: detect the log level and normalize the message so that
//! lines that differ only in ids, numbers or timestamps group together.

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" | "notice" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" | "err" | "severe" => Some(Level::Error),
            "fatal" | "critical" | "crit" | "panic" | "panicked" | "emerg" | "alert" => {
                Some(Level::Fatal)
            }
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
        }
    }
}

static LEVEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(?:^|[\s\[|("]|level=|"level":\s*")(trace|debug|info|notice|warn(?:ing)?|err(?:or)?|severe|fatal|crit(?:ical)?|panic(?:ked)?|emerg|alert)(?:$|[\s\]|):,"])"#,
    )
    .unwrap()
});

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?")
        .unwrap()
});

/// Ordered replacements: most specific patterns first.
static MASKS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?",
            "",
        ),
        (
            r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            "<uuid>",
        ),
        (r"[\w.+-]+@[\w-]+\.[\w.]+", "<email>"),
        (r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b", "<ip>"),
        (r"(?i)\b0x[0-9a-f]+\b", "<hex>"),
        (r"(?i)\b[0-9a-f]{16,}\b", "<hash>"),
        (r#""[^"]*""#, "\"<str>\""),
        (r"\b\d+(?:\.\d+)?(?:ms|s|kb|mb|gb|%)?\b", "<n>"),
    ]
    .into_iter()
    .map(|(p, r)| (Regex::new(p).unwrap(), r))
    .collect()
});

static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub level: Option<Level>,
    pub timestamp: Option<String>,
    /// The line with the level and timestamp removed.
    pub message: String,
    /// The message with variable parts masked; equal signatures are grouped.
    pub signature: String,
}

pub fn parse_line(line: &str) -> Entry {
    let level_match = LEVEL.captures(line).and_then(|c| c.get(1));
    let level = level_match.and_then(|m| Level::parse(m.as_str()));
    let timestamp = TIMESTAMP.find(line).map(|m| m.as_str().to_string());

    let mut message = line.to_string();
    if let Some(m) = level_match {
        message.replace_range(m.range(), "");
    }
    if let Some(ts) = &timestamp {
        message = message.replacen(ts.as_str(), "", 1);
    }
    let message = tidy(&message);
    let signature = normalize(&message);
    Entry {
        level,
        timestamp,
        message,
        signature,
    }
}

pub fn normalize(message: &str) -> String {
    let mut out = message.to_string();
    for (re, replacement) in MASKS.iter() {
        out = re.replace_all(&out, *replacement).into_owned();
    }
    tidy(&out)
}

fn tidy(s: &str) -> String {
    let collapsed = SPACES.replace_all(s.trim(), " ");
    collapsed
        .trim_matches(|c: char| {
            c == '[' || c == ']' || c == ':' || c == '|' || c == '-' || c.is_whitespace()
        })
        .replace("[] ", "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_level_formats() {
        let cases = [
            ("2026-01-01 10:00:00 ERROR db timeout", Level::Error),
            ("[WARN] disk almost full", Level::Warn),
            ("level=info msg=started", Level::Info),
            (r#"{"level":"fatal","msg":"boom"}"#, Level::Fatal),
            ("Jan 01 host app[42]: warning: retrying", Level::Warn),
            ("thread 'main' panicked at src/main.rs:3", Level::Fatal),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_line(line).level, Some(expected), "{line}");
        }
    }

    #[test]
    fn no_level_for_plain_text() {
        assert_eq!(parse_line("just some output").level, None);
        // "error" inside a word is not a level
        assert_eq!(parse_line("terrorism_filter loaded").level, None);
    }

    #[test]
    fn extracts_timestamp_and_strips_it() {
        let e = parse_line("2026-03-04T05:06:07.123Z ERROR payment failed");
        assert_eq!(e.timestamp.as_deref(), Some("2026-03-04T05:06:07.123Z"));
        assert_eq!(e.message, "payment failed");
    }

    #[test]
    fn similar_lines_share_a_signature() {
        let a = parse_line("ERROR user 1042 failed login from 10.0.0.7 after 350ms");
        let b = parse_line("ERROR user 77 failed login from 192.168.1.20 after 12ms");
        assert_eq!(a.signature, b.signature);
        assert_eq!(a.signature, "user <n> failed login from <ip> after <n>");
    }

    #[test]
    fn masks_ids_hashes_and_strings() {
        let s = normalize(
            r#"order 3f2b8c1e-9a4d-4b6e-8f00-123456789abc for a@b.com key="secret" ptr 0xdeadbeef"#,
        );
        assert_eq!(s, r#"order <uuid> for <email> key="<str>" ptr <hex>"#);
    }
}
