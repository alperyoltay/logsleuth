//! Aggregation: count lines per level and group similar messages.

use crate::parse::{Entry, Level};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub level: Level,
    pub signature: String,
    pub count: usize,
    /// One real line from the group, so the reader sees concrete values.
    pub example: String,
    pub first_line: usize,
    pub last_line: usize,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub total_lines: usize,
    pub by_level: BTreeMap<Level, usize>,
    pub unleveled: usize,
    pub groups: Vec<Group>,
}

#[derive(Default)]
pub struct Analyzer {
    min_level: Option<Level>,
    total: usize,
    by_level: BTreeMap<Level, usize>,
    unleveled: usize,
    groups: HashMap<(Level, String), Group>,
}

impl Analyzer {
    /// Only lines at `min_level` or above are grouped; all lines are counted.
    pub fn new(min_level: Level) -> Self {
        Analyzer {
            min_level: Some(min_level),
            ..Default::default()
        }
    }

    pub fn add(&mut self, line_no: usize, raw: &str, entry: Entry) {
        self.total += 1;
        let Some(level) = entry.level else {
            self.unleveled += 1;
            return;
        };
        *self.by_level.entry(level).or_default() += 1;
        if self.min_level.is_some_and(|min| level < min) {
            return;
        }
        self.groups
            .entry((level, entry.signature.clone()))
            .and_modify(|g| {
                g.count += 1;
                g.last_line = line_no;
                if entry.timestamp.is_some() {
                    g.last_seen = entry.timestamp.clone();
                }
            })
            .or_insert_with(|| Group {
                level,
                signature: entry.signature,
                count: 1,
                example: raw.trim().to_string(),
                first_line: line_no,
                last_line: line_no,
                first_seen: entry.timestamp.clone(),
                last_seen: entry.timestamp,
            });
    }

    /// Groups sorted by severity, then frequency, then first appearance.
    pub fn finish(self, top: usize) -> Report {
        let mut groups: Vec<Group> = self.groups.into_values().collect();
        groups.sort_by(|a, b| {
            b.level
                .cmp(&a.level)
                .then(b.count.cmp(&a.count))
                .then(a.first_line.cmp(&b.first_line))
        });
        groups.truncate(top);
        Report {
            total_lines: self.total,
            by_level: self.by_level,
            unleveled: self.unleveled,
            groups,
        }
    }
}

impl Report {
    pub fn count_at_least(&self, level: Level) -> usize {
        self.by_level.range(level..).map(|(_, n)| n).sum()
    }

    pub fn to_text(&self) -> String {
        let mut out = format!("{} lines", self.total_lines);
        for (level, n) in &self.by_level {
            out.push_str(&format!(" · {} {}", n, level.as_str()));
        }
        if self.unleveled > 0 {
            out.push_str(&format!(" · {} without level", self.unleveled));
        }
        out.push('\n');

        if self.groups.is_empty() {
            out.push_str("\nNo matching problems found.\n");
            return out;
        }
        for (i, g) in self.groups.iter().enumerate() {
            out.push_str(&format!(
                "\n{:>2}. [{}] x{}  {}\n",
                i + 1,
                g.level.as_str(),
                g.count,
                g.signature
            ));
            out.push_str(&format!(
                "    e.g. line {}: {}\n",
                g.first_line,
                truncate(&g.example, 160)
            ));
            match (&g.first_seen, &g.last_seen) {
                (Some(first), Some(last)) if g.count > 1 => out.push_str(&format!(
                    "    seen {first} → {last} (lines {}–{})\n",
                    g.first_line, g.last_line
                )),
                _ if g.count > 1 => {
                    out.push_str(&format!("    lines {}–{}\n", g.first_line, g.last_line))
                }
                _ => {}
            }
        }
        out
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_line;

    fn analyze(lines: &[&str], min: Level, top: usize) -> Report {
        let mut a = Analyzer::new(min);
        for (i, line) in lines.iter().enumerate() {
            a.add(i + 1, line, parse_line(line));
        }
        a.finish(top)
    }

    const LOG: &[&str] = &[
        "2026-01-01 10:00:00 INFO server started",
        "2026-01-01 10:00:01 ERROR db timeout after 3000ms",
        "2026-01-01 10:00:02 WARN slow request 812ms",
        "2026-01-01 10:00:03 ERROR db timeout after 3100ms",
        "2026-01-01 10:00:04 FATAL out of memory",
        "plain line",
    ];

    #[test]
    fn counts_levels_and_groups_similar_errors() {
        let r = analyze(LOG, Level::Warn, 10);
        assert_eq!(r.total_lines, 6);
        assert_eq!(r.unleveled, 1);
        assert_eq!(r.by_level[&Level::Error], 2);
        assert_eq!(r.groups.len(), 3);

        // FATAL first, then the repeated ERROR, then WARN
        assert_eq!(r.groups[0].level, Level::Fatal);
        let db = &r.groups[1];
        assert_eq!((db.count, db.first_line, db.last_line), (2, 2, 4));
        assert_eq!(db.last_seen.as_deref(), Some("2026-01-01 10:00:03"));
        assert_eq!(r.groups[2].level, Level::Warn);
    }

    #[test]
    fn min_level_and_top_limit() {
        let r = analyze(LOG, Level::Error, 1);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.count_at_least(Level::Error), 3);
        assert_eq!(r.count_at_least(Level::Warn), 4);
    }

    #[test]
    fn text_output_is_readable() {
        let text = analyze(LOG, Level::Warn, 10).to_text();
        assert!(
            text.starts_with("6 lines · 1 INFO · 1 WARN · 2 ERROR · 1 FATAL · 1 without level")
        );
        assert!(text.contains("[ERROR] x2  db timeout after <n>"));
        assert!(text.contains("seen 2026-01-01 10:00:01 → 2026-01-01 10:00:03 (lines 2–4)"));
    }

    #[test]
    fn empty_input() {
        let r = analyze(&[], Level::Warn, 10);
        assert!(r.to_text().contains("No matching problems found."));
    }
}
