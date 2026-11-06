// SPDX-License-Identifier: GPL-3.0-or-later
//! A careful line editor for the runtime's ini files (pkp.ini, controls.ini).
//!
//! It never rebuilds the file from a parsed model. Lines are kept as they are; `set` changes only the
//! value text of one `key = value` line (keeping the key spelling, the spacing and any trailing comment)
//! or inserts a new line. Comments, blank lines, unknown keys and the line-ending style survive.
//! Section and key names are case-insensitive, as in the runtime (nx/runtime/docs/CONFIG.md).

pub struct Ini {
    lines: Vec<String>,
    eol: &'static str,
    final_eol: bool,
}

fn section_name(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('[') {
        if let Some(end) = t.find(']') {
            return Some(t[1..end].trim().to_ascii_lowercase());
        }
    }
    None
}

/// Index of the `=` of a key line, or None for blanks, comments, sections and lines without `=`.
fn key_eq(line: &str) -> Option<usize> {
    let t = line.trim_start();
    if t.is_empty() || t.starts_with(';') || t.starts_with('#') || t.starts_with('[') {
        return None;
    }
    line.find('=')
}

/// Splits the text after `=` (leading blanks already removed) into (value, tail), where the tail is
/// the blanks and comment that follow the value.
fn split_value(rest: &str) -> (&str, &str) {
    if rest.starts_with('"') {
        if let Some(close) = rest[1..].find('"') {
            let end = close + 2;
            return (&rest[..end], &rest[end..]);
        }
    }
    let b = rest.as_bytes();
    for i in 1..b.len() {
        if (b[i] == b';' || b[i] == b'#') && (b[i - 1] == b' ' || b[i - 1] == b'\t') {
            let mut s = i;
            while s > 0 && (b[s - 1] == b' ' || b[s - 1] == b'\t') {
                s -= 1;
            }
            return (&rest[..s], &rest[s..]);
        }
    }
    let trimmed = rest.trim_end();
    (trimmed, &rest[trimmed.len()..])
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// A value is written quoted when the runtime would otherwise misread it.
fn format_value(v: &str) -> String {
    let v: String = v.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    let needs = v != v.trim()
        || v.contains(" ;")
        || v.contains(" #")
        || v.contains("\t;")
        || v.contains("\t#")
        || v.starts_with('"');
    if needs && !v.contains('"') {
        format!("\"{}\"", v)
    } else {
        v
    }
}

impl Ini {
    pub fn parse(text: &str) -> Ini {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let final_eol = text.ends_with('\n');
        let lines = text.lines().map(|l| l.to_string()).collect();
        Ini { lines, eol, final_eol }
    }

    pub fn empty() -> Ini {
        Ini { lines: Vec::new(), eol: "\r\n", final_eol: true }
    }

    /// Line index of the last `key` in `section`, if any.
    fn find(&self, section: &str, key: &str) -> Option<usize> {
        let (section, key) = (section.to_ascii_lowercase(), key.to_ascii_lowercase());
        let mut cur = String::new();
        let mut hit = None;
        for (i, line) in self.lines.iter().enumerate() {
            if let Some(s) = section_name(line) {
                cur = s;
                continue;
            }
            if cur == section {
                if let Some(eq) = key_eq(line) {
                    if line[..eq].trim().to_ascii_lowercase() == key {
                        hit = Some(i);
                    }
                }
            }
        }
        hit
    }

    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let i = self.find(section, key)?;
        let line = &self.lines[i];
        let eq = key_eq(line)?;
        let rest = line[eq + 1..].trim_start();
        Some(unquote(split_value(rest).0))
    }

    /// Every `section.key` (lower case) with its value; a repeated key keeps the last value.
    pub fn all(&self) -> Vec<(String, String)> {
        let mut cur = String::new();
        let mut out: Vec<(String, String)> = Vec::new();
        for line in &self.lines {
            if let Some(s) = section_name(line) {
                cur = s;
                continue;
            }
            if let Some(eq) = key_eq(line) {
                if cur.is_empty() {
                    continue;
                }
                let k = format!("{}.{}", cur, line[..eq].trim().to_ascii_lowercase());
                let v = unquote(split_value(line[eq + 1..].trim_start()).0);
                if let Some(e) = out.iter_mut().find(|e| e.0 == k) {
                    e.1 = v;
                } else {
                    out.push((k, v));
                }
            }
        }
        out
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let value = format_value(value);
        if let Some(i) = self.find(section, key) {
            let line = &self.lines[i];
            let eq = key_eq(line).unwrap();
            let after = &line[eq + 1..];
            let blanks = after.len() - after.trim_start().len();
            let (_, tail) = split_value(&after[blanks..]);
            let new_line = format!("{}{}{}{}", &line[..=eq], &after[..blanks], value, tail);
            self.lines[i] = new_line;
            return;
        }
        let new_line = format!("{} = {}", key, value);
        // insert after the last non-blank line of the section, or start the section at the end
        let target = section.to_ascii_lowercase();
        let mut cur = String::new();
        let mut last_in_section: Option<usize> = None;
        let mut found_section = false;
        for (i, line) in self.lines.iter().enumerate() {
            if let Some(s) = section_name(line) {
                cur = s;
                if cur == target {
                    found_section = true;
                    last_in_section = Some(i);
                }
                continue;
            }
            if cur == target && !line.trim().is_empty() {
                last_in_section = Some(i);
            }
        }
        if found_section {
            self.lines.insert(last_in_section.unwrap() + 1, new_line);
        } else {
            if self.lines.last().map_or(false, |l| !l.trim().is_empty()) {
                self.lines.push(String::new());
            }
            self.lines.push(format!("[{}]", section));
            self.lines.push(new_line);
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = self.lines.join(self.eol);
        if self.final_eol {
            s.push_str(self.eol);
        }
        s
    }
}
