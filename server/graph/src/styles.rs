//! Design-token definitions and usages found in web code: CSS custom properties, Tailwind
//! utilities and raw colors (port of `styles.mjs`). Dart tokens come from the Dart parser instead.

use crate::jsre::{JsRegex, JsStr};
use crate::re;
use indexmap::IndexMap;
use std::sync::LazyLock;

/// A token definition (`--color-primary: #2563eb` → name `--color-primary`).
#[derive(Clone, Debug, PartialEq)]
pub struct StyleDef {
    pub name: String,
    pub value: String,
    pub line: i64,
}

/// A token use: `form` = `var` | `tw` | `hex` (| `dart` from the Dart parser).
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRef {
    pub form: String,
    pub name: String,
    pub line: i64,
    pub count: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styles {
    pub defs: Vec<StyleDef>,
    pub refs: Vec<StyleRef>,
}

impl Styles {
    /// The JS shape: `{ defs: [{ name, value, line }], refs: [{ form, name, line, count }] }`.
    pub fn to_json(&self) -> serde_json::Value {
        let defs: Vec<_> = self.defs.iter().map(|d| serde_json::json!({ "name": d.name, "value": d.value, "line": d.line })).collect();
        let refs: Vec<_> = self
            .refs
            .iter()
            .map(|r| serde_json::json!({ "form": r.form, "name": r.name, "line": r.line, "count": r.count }))
            .collect();
        serde_json::json!({ "defs": defs, "refs": refs })
    }
}

/// Tailwind utility prefix → the @theme namespace it reads (Tailwind v4 naming).
pub static TW_NAMESPACE: LazyLock<Vec<(JsRegex, &'static str)>> = LazyLock::new(|| {
    vec![
        (
            JsRegex::new(
                r"^(bg|text|border|border-[trblxy]|ring|ring-offset|outline|fill|stroke|from|via|to|decoration|divide|placeholder|caret|accent|shadow|inset-shadow)$",
                "",
            ),
            "color",
        ),
        (JsRegex::new(r"^(rounded|rounded-[trbl]{1,2}|rounded-[se][se]?)$", ""), "radius"),
        (
            JsRegex::new(
                r"^(p|px|py|pt|pr|pb|pl|ps|pe|m|mx|my|mt|mr|mb|ml|ms|me|gap|gap-x|gap-y|space-x|space-y|w|h|min-w|min-h|max-w|max-h|size|inset|top|right|bottom|left|translate-x|translate-y|scroll-m|scroll-p)$",
                "",
            ),
            "spacing",
        ),
        (JsRegex::new(r"^(shadow)$", ""), "shadow"),
        (JsRegex::new(r"^(text)$", ""), "text"),
        (JsRegex::new(r"^(font)$", ""), "font"),
        (JsRegex::new(r"^(leading)$", ""), "leading"),
    ]
});

fn tw_util() -> &'static JsRegex {
    re!(
        r"(?<![\w-])(?:[a-z0-9-]+:)*-?((?:inset-shadow|ring-offset|border-[trblxy]|rounded-[trbl]{1,2}|rounded-[se]{1,2}|space-[xy]|gap-[xy]|min-[wh]|max-[wh]|translate-[xy]|scroll-[mp]|bg|text|border|ring|outline|fill|stroke|from|via|to|decoration|divide|placeholder|caret|accent|shadow|rounded|p[xytrblse]?|m[xytrblse]?|gap|w|h|size|inset|top|right|bottom|left|font|leading))-([a-z][a-z0-9-]*?)(?:\/\d+)?(?![\w-])",
        "g"
    )
}

/// 1-based line of a UTF-16 index (same as counting `\n` before it).
pub(crate) struct Lines(Vec<usize>);

impl Lines {
    pub(crate) fn new(text: &JsStr) -> Self {
        Lines(text.units().iter().enumerate().filter(|(_, u)| **u == 10).map(|(i, _)| i).collect())
    }
    pub(crate) fn at(&self, index: usize) -> i64 {
        self.0.partition_point(|&i| i < index) as i64 + 1
    }
}

/// `scanStyles(text, path)` → definitions and usages (refs are grouped per form+name with a count).
pub fn scan_styles(text: &str, path: &str) -> Styles {
    let is_css = re!(r"\.(css|scss|sass|less)$").test_str(path);
    if !is_css && !re!(r"\.(tsx?|jsx?|mjs|cjs|vue|svelte|astro|html)$").test_str(path) {
        return Styles::default();
    }
    let text = JsStr::new(text);
    let lines = Lines::new(&text);
    let mut defs = Vec::new();
    let mut refs: IndexMap<String, StyleRef> = IndexMap::new();
    let mut add = |form: &str, name: String, index: usize| {
        let key = format!("{form}|{name}");
        if let Some(r) = refs.get_mut(&key) {
            r.count += 1;
        } else {
            refs.insert(
                key,
                StyleRef {
                    form: form.into(),
                    name,
                    line: lines.at(index),
                    count: 1,
                },
            );
        }
    };
    let mut def_lines = std::collections::HashSet::new();
    let util = tw_util();
    if is_css {
        for m in re!(r"(?<![\w-])--([A-Za-z0-9_-]+)\s*:\s*([^;{}]+);", "g").all(&text) {
            let line = lines.at(m.index);
            def_lines.insert(line);
            let value = m.get(&text, 2).unwrap().trim();
            defs.push(StyleDef {
                name: format!("--{}", m.get(&text, 1).unwrap()),
                value: value.slice(0, 80).to_string(),
                line,
            });
        }
        // @apply bg-primary … inside CSS
        for m in re!(r"@apply\s+([^;]+);", "g").all(&text) {
            let body = m.get(&text, 1).unwrap();
            for u in util.all(&body) {
                add("tw", format!("{}-{}", u.get(&body, 1).unwrap(), u.get(&body, 2).unwrap()), m.index);
            }
        }
    }
    for m in re!(r"var\(\s*--([A-Za-z0-9_-]+)", "g").all(&text) {
        add("var", format!("--{}", m.get(&text, 1).unwrap()), m.index);
    }
    if !is_css {
        // Only inside string-ish class lists: className="…", cn("…"), cva("…"), tw`…`, class="…"
        for s in re!(r#"(["'`])((?:(?!\1)[^\\\n]|\\.){2,400}?)\1"#, "g").all(&text) {
            let body = s.get(&text, 2).unwrap();
            if !re!(r"[a-z]-[a-z]").test(&body) {
                continue;
            }
            for u in util.all(&body) {
                add("tw", format!("{}-{}", u.get(&body, 1).unwrap(), u.get(&body, 2).unwrap()), s.index);
            }
        }
    }
    // Raw colors written in code (not token definitions)
    for m in re!(r"#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3})(?![0-9a-zA-Z_-])", "g").all(&text) {
        if is_css && def_lines.contains(&lines.at(m.index)) {
            continue;
        }
        if !is_css && !re!(r#"["'`]\s*#|:\s*["'`]?#|\(\s*["'`]?#"#).test(&text.slice(m.index.saturating_sub(3), m.index + 1)) {
            continue;
        }
        add("hex", m.whole(&text).to_lower().to_string(), m.index);
    }
    Styles {
        defs,
        refs: refs.into_values().collect(),
    }
}

/// Tailwind `bg-primary` → candidate CSS variables it would read (`--color-primary`).
pub fn tw_to_vars(util: &str) -> Vec<String> {
    let u = JsStr::new(util);
    let mut from = 0;
    while let Some(i) = u.index_of("-", from) {
        let prefix = u.slice(0, i);
        let value = u.slice_from(i + 1).to_string();
        let hits: Vec<String> = TW_NAMESPACE.iter().filter(|(re, _)| re.test(&prefix)).map(|(_, ns)| format!("--{ns}-{value}")).collect();
        if !hits.is_empty() {
            return hits;
        }
        from = i + 1;
    }
    Vec::new()
}

/// Classes that hold design tokens in Dart (same rule as the Dart parser).
pub fn dart_token_class() -> &'static JsRegex {
    re!(r"(Colou?rs?|Palette|Spacing|Spaces?|Radius|Radii|Sizes?|Shadows?|Elevations?|Typography|TextStyles?|Fonts?|Tokens?|Insets|Gaps|Durations|Breakpoints|Theme)$")
}

/// Comparable color: `#2563EB`, `#2563ebff`, `Color(0xFF2563EB)`, `0xFF2563EB` → `#2563eb`
/// (alpha kept only when not opaque).
pub fn norm_color(v: Option<&str>) -> Option<String> {
    let v = v?;
    if v.is_empty() {
        return None;
    }
    let s = JsStr::new(v).trim();
    if let Some(m) = re!(r"0x([0-9a-f]{8})", "i").exec(&s) {
        let d = m.get(&s, 1).unwrap().to_string();
        let (a, rgb) = (&d[..2], &d[2..]);
        let alpha = if a.to_lowercase() == "ff" { "" } else { a };
        return Some(format!("#{rgb}{alpha}").to_lowercase());
    }
    let m = re!(r"^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$", "i").exec(&s)?;
    let mut h = m.get(&s, 1).unwrap().to_string().to_lowercase();
    if h.len() == 3 {
        h = h.chars().flat_map(|c| [c, c]).collect();
    }
    if h.len() == 8 && h.ends_with("ff") {
        h.truncate(6);
    }
    Some(format!("#{h}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_and_tailwind() {
        let s = scan_styles(
            ":root { --color-primary: #2563EB; }\n.a { @apply bg-primary text-sm; color: var(--color-primary); border: 1px solid #FFF; }",
            "app.css",
        );
        assert_eq!(
            s.defs,
            vec![StyleDef {
                name: "--color-primary".into(),
                value: "#2563EB".into(),
                line: 1
            }]
        );
        let names: Vec<_> = s.refs.iter().map(|r| format!("{}:{}", r.form, r.name)).collect();
        assert_eq!(names, ["tw:bg-primary", "tw:text-sm", "var:--color-primary", "hex:#fff"]);
        let t = scan_styles("<div className=\"bg-primary/50 hover:text-muted-foreground p-4\" style={{color: '#abc'}} />", "a.tsx");
        let names: Vec<_> = t.refs.iter().map(|r| format!("{}:{}", r.form, r.name)).collect();
        assert_eq!(names, ["tw:bg-primary", "tw:text-muted-foreground", "hex:#abc"]);
        assert_eq!(tw_to_vars("bg-primary"), ["--color-primary"]);
        assert_eq!(tw_to_vars("text-lg"), ["--color-lg", "--text-lg"]);
        assert_eq!(norm_color(Some("Color(0xFF2563EB)")).as_deref(), Some("#2563eb"));
        assert_eq!(norm_color(Some("#ABCDEF80")).as_deref(), Some("#abcdef80"));
        assert_eq!(norm_color(Some("red")), None);
    }
}
