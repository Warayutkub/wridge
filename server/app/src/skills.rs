//! Port of skills.mjs. Skills are markdown files in /skills with a small front matter:
//!   ---
//!   name: ui-visual
//!   description: When to load it (one line — shown to the AI in the skill list).
//!   area: design | code | both (optional — design sessions list design skills, code sessions code skills)
//!   ---
//! They are read on every request, so adding or editing a file needs no restart.

use std::cmp::Ordering;
use std::path::Path;

use crate::jsjson::js_trim;

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// design | code | both — which sessions list it ("" when the file doesn't say)
    pub area: String,
    pub body: String,
    pub file: String,
    pub tokens: u64,
}

fn parse(text: &str, file: &str) -> Skill {
    let re = regress::Regex::new(r"^---\r?\n([\s\S]*?)\r?\n---\r?\n?").unwrap();
    let m = re.find(text);
    let mut meta: Vec<(String, String)> = vec![];
    if let Some(m) = &m {
        let group = m.group(1).map(|r| &text[r]).unwrap_or("");
        let split = regress::Regex::new(r"\r?\n").unwrap();
        let mut last = 0;
        let mut lines = vec![];
        for s in split.find_iter(group) {
            lines.push(&group[last..s.start()]);
            last = s.end();
        }
        lines.push(&group[last..]);
        for line in lines {
            if let Some(i) = line.find(':') {
                if i > 0 {
                    let key = js_trim(&line[..i]).to_string();
                    let value = js_trim(&line[i + 1..]).to_string();
                    match meta.iter_mut().find(|(k, _)| *k == key) {
                        Some(entry) => entry.1 = value,
                        None => meta.push((key, value)),
                    }
                }
            }
        }
    }
    let get = |k: &str| meta.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let name = get("name");
    Skill {
        name: if name.is_empty() { file.strip_suffix(".md").unwrap_or(file).to_string() } else { name },
        description: get("description"),
        area: get("area"),
        body: match &m {
            Some(m) => js_trim(&text[m.end()..]).to_string(),
            None => js_trim(text).to_string(),
        },
        file: file.to_string(),
        tokens: estimate_tokens(text),
    }
}

/// `Math.round` (rounds .5 up, toward +∞).
pub fn js_round(x: f64) -> f64 {
    let r = x.floor();
    if x - r >= 0.5 {
        r + 1.0
    } else {
        r
    }
}

/// Rough token count for display (~4 characters per token for Latin text, ~1.5 for Thai and symbols),
/// rounded to tens — close enough to compare skills, not a billing number.
pub fn estimate_tokens(text: &str) -> u64 {
    let mut ascii = 0u64;
    let mut other = 0u64;
    for ch in text.chars() {
        if (ch as u32) < 128 {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    let v = js_round((ascii as f64 / 4.0 + other as f64 / 1.5) / 10.0) * 10.0;
    v.max(10.0) as u64
}

pub fn load_skills(dir: &Path) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    // readdirSync order (alphabetical on Windows/NTFS); the sort below decides the final order anyway.
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".md"))
        .collect();
    files.sort();
    let mut skills: Vec<Skill> = files
        .iter()
        .filter_map(|f| {
            let bytes = std::fs::read(dir.join(f)).ok()?;
            Some(parse(&String::from_utf8_lossy(&bytes), f))
        })
        .collect();
    skills.sort_by(|a, b| locale_compare(&a.name, &b.name));
    skills
}

pub fn skill_index(skills: &[Skill]) -> String {
    skills.iter().map(|s| format!("- `{}`: {}", s.name, s.description)).collect::<Vec<_>>().join("\n")
}

/// Primary weight of a character in the CLDR root collation (what `localeCompare` uses): punctuation and
/// symbols first, then digits, then letters case-insensitively. Good for skill names and file names.
fn primary(c: char) -> (u32, u32) {
    const PUNCT: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if c.is_whitespace() {
        return (0, c as u32);
    }
    if let Some(i) = PUNCT.find(c) {
        return (1, i as u32);
    }
    if c.is_ascii_digit() {
        return (2, c as u32);
    }
    if c.is_alphabetic() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        return (3, lower as u32);
    }
    (1, 1000 + c as u32)
}

/// `a.localeCompare(b)` approximated with the root collation's primary order, then lowercase before
/// uppercase, then code units.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let pa: Vec<(u32, u32)> = a.chars().map(primary).collect();
    let pb: Vec<(u32, u32)> = b.chars().map(primary).collect();
    match pa.cmp(&pb) {
        Ordering::Equal => {}
        o => return o,
    }
    // tertiary: lowercase sorts before uppercase
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            let (xl, yl) = (x.is_lowercase(), y.is_lowercase());
            if xl != yl {
                return if xl { Ordering::Less } else { Ordering::Greater };
            }
        }
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from legacy-node/test/skills.test.mjs
    #[test]
    fn load_skills_reads_front_matter_file_name_and_size_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.md"), "---\nname: beta\ndescription: When coding\narea: code\n---\n# Beta\nBody text").unwrap();
        std::fs::write(dir.path().join("a.md"), "# No front matter").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let skills = load_skills(dir.path());
        let got: Vec<(&str, &str, &str)> = skills.iter().map(|s| (s.name.as_str(), s.area.as_str(), s.file.as_str())).collect();
        assert_eq!(got, vec![("a", "", "a.md"), ("beta", "code", "b.md")]);
        assert_eq!(skills[1].body, "# Beta\nBody text");
        assert!(skills[1].tokens >= 10);
    }

    #[test]
    fn estimate_tokens_counts_thai_heavier_than_latin_and_rounds_to_tens() {
        assert_eq!(estimate_tokens(&"a".repeat(400)), 100);
        assert_eq!(estimate_tokens(&"ก".repeat(150)), 100);
        assert_eq!(estimate_tokens(""), 10);
    }

    #[test]
    fn a_missing_skills_folder_gives_an_empty_list() {
        assert!(load_skills(&std::env::temp_dir().join("wara-no-such-dir-xyz")).is_empty());
    }

    #[test]
    fn locale_order() {
        let mut v = vec!["ui-visual", "uikit", "motion-craft", "motion", "Alpha", "alpha", "b2", "b10"];
        v.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(v, vec!["alpha", "Alpha", "b10", "b2", "motion", "motion-craft", "ui-visual", "uikit"]);
    }
}
