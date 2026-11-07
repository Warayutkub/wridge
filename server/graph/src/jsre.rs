//! JavaScript strings and regular expressions with V8 semantics.
//!
//! The Node code scans source text with JS regexes and JS string indices (UTF-16 code units).
//! [`JsStr`] holds text as UTF-16 and [`JsRegex`] runs ECMAScript regexes (via `regress`, in
//! non-unicode UCS-2 mode, like a JS regex without the `u` flag) on it, so match positions,
//! `\s`/`\w`/`\b`, case folding, `.slice()` and `.length` all agree with the original.
//!
//! Use the [`re!`](crate::re) macro for a lazily compiled static regex: `re!(r"\bfoo\b", "i")`.

use std::fmt;

/// A JS string: UTF-16 code units.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsStr(pub Vec<u16>);

impl JsStr {
    pub fn new(s: &str) -> Self {
        JsStr(s.encode_utf16().collect())
    }
    pub fn from_units(u: &[u16]) -> Self {
        JsStr(u.to_vec())
    }
    pub fn units(&self) -> &[u16] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// `s[i]` / `s.charCodeAt(i)`.
    pub fn at(&self, i: usize) -> Option<u16> {
        self.0.get(i).copied()
    }
    /// `s[i] === c` for an ASCII char.
    pub fn is(&self, i: usize, c: char) -> bool {
        self.at(i) == Some(c as u16)
    }
    /// `s.slice(start, end)` (non-negative indices, clamped).
    pub fn slice(&self, start: usize, end: usize) -> JsStr {
        let end = end.min(self.0.len());
        if start >= end {
            return JsStr::default();
        }
        JsStr(self.0[start..end].to_vec())
    }
    /// `s.slice(start, end)` with JS rules for negative indices (counted from the end).
    pub fn slice_js(&self, start: isize, end: isize) -> JsStr {
        let len = self.0.len() as isize;
        let fix = |i: isize| if i < 0 { (len + i).max(0) } else { i.min(len) };
        let (a, b) = (fix(start), fix(end));
        if a >= b {
            return JsStr::default();
        }
        JsStr(self.0[a as usize..b as usize].to_vec())
    }
    /// `s.slice(start)`.
    pub fn slice_from(&self, start: usize) -> JsStr {
        self.slice(start, self.0.len())
    }
    /// `s.indexOf(needle, from)`.
    pub fn index_of(&self, needle: &str, from: usize) -> Option<usize> {
        let n: Vec<u16> = needle.encode_utf16().collect();
        self.index_of_units(&n, from)
    }
    pub fn index_of_units(&self, n: &[u16], from: usize) -> Option<usize> {
        let h = &self.0;
        if n.is_empty() {
            return Some(from.min(h.len()));
        }
        if n.len() > h.len() {
            return None;
        }
        (from..=h.len() - n.len()).find(|&i| &h[i..i + n.len()] == n)
    }
    /// `s.startsWith(prefix, pos)`.
    pub fn starts_with_at(&self, prefix: &[u16], pos: usize) -> bool {
        pos <= self.0.len() && self.0[pos..].starts_with(prefix)
    }
    pub fn starts_with(&self, prefix: &str) -> bool {
        let p: Vec<u16> = prefix.encode_utf16().collect();
        self.0.starts_with(&p)
    }
    pub fn ends_with(&self, suffix: &str) -> bool {
        let p: Vec<u16> = suffix.encode_utf16().collect();
        self.0.ends_with(&p)
    }
    pub fn eq_str(&self, s: &str) -> bool {
        self.0.iter().copied().eq(s.encode_utf16())
    }
    /// `s.trim()`.
    pub fn trim(&self) -> JsStr {
        let sp = |u: &u16| crate::js::is_js_space(*u as u32);
        let start = self.0.iter().position(|u| !sp(u)).unwrap_or(self.0.len());
        let end = self.0.iter().rposition(|u| !sp(u)).map(|i| i + 1).unwrap_or(start);
        self.slice(start, end)
    }
    /// `s.toLowerCase()`.
    pub fn to_lower(&self) -> JsStr {
        JsStr::new(&self.to_string().to_lowercase())
    }
    /// `s.split(sep)` with a string separator.
    pub fn split(&self, sep: &str) -> Vec<JsStr> {
        let n: Vec<u16> = sep.encode_utf16().collect();
        if n.is_empty() {
            return self.0.iter().map(|u| JsStr(vec![*u])).collect();
        }
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(i) = self.index_of_units(&n, from) {
            out.push(self.slice(from, i));
            from = i + n.len();
        }
        out.push(self.slice_from(from));
        out
    }
    /// Concatenation.
    pub fn concat(&self, other: &JsStr) -> JsStr {
        let mut v = self.0.clone();
        v.extend_from_slice(&other.0);
        JsStr(v)
    }
    /// Number of `\n` in `s.slice(0, index)` plus one: the 1-based line of `index`.
    pub fn line_at(&self, index: usize) -> usize {
        self.0[..index.min(self.0.len())].iter().filter(|u| **u == 10).count() + 1
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf16_lossy(&self.0))
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string())
    }
}

impl From<&str> for JsStr {
    fn from(s: &str) -> Self {
        JsStr::new(s)
    }
}

/// One match: `m.index`, `m[0].length` and the capture groups (UTF-16 ranges).
#[derive(Clone, Debug)]
pub struct Caps {
    pub index: usize,
    pub end: usize,
    groups: Vec<Option<(usize, usize)>>,
}

impl Caps {
    /// Range of group `i` (0 = whole match), `None` when it did not participate (`undefined`).
    pub fn range(&self, i: usize) -> Option<(usize, usize)> {
        if i == 0 {
            Some((self.index, self.end))
        } else {
            self.groups.get(i - 1).copied().flatten()
        }
    }
    /// `m[i]` as a JS string.
    pub fn get(&self, s: &JsStr, i: usize) -> Option<JsStr> {
        self.range(i).map(|(a, b)| s.slice(a, b))
    }
    /// `m[i]` as a Rust string.
    pub fn str(&self, s: &JsStr, i: usize) -> Option<String> {
        self.get(s, i).map(|x| x.to_string())
    }
    /// `m[0]`.
    pub fn whole(&self, s: &JsStr) -> JsStr {
        s.slice(self.index, self.end)
    }
    /// `m[0].length`.
    pub fn len(&self) -> usize {
        self.end - self.index
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.index
    }
}

/// A compiled ECMAScript regex (flags `i`, `m`, `s`, `u`; `g` only changes which method you call).
/// Without `u` the text is matched as UCS-2 code units (like V8); with `u` surrogate pairs are one
/// code point and `\p{…}` classes work.
pub struct JsRegex {
    re: regress::Regex,
    unicode: bool,
}

impl JsRegex {
    /// Compiles `pattern` (JS source, as between the slashes) with JS `flags`. Panics on invalid
    /// syntax: patterns come from the code, not from users. Use [`JsRegex::try_new`] otherwise.
    pub fn new(pattern: &str, flags: &str) -> JsRegex {
        Self::try_new(pattern, flags).unwrap_or_else(|e| panic!("bad regex /{pattern}/{flags}: {e}"))
    }

    pub fn try_new(pattern: &str, flags: &str) -> Result<JsRegex, regress::Error> {
        let f: String = flags.chars().filter(|c| "imsu".contains(*c)).collect();
        Ok(JsRegex {
            re: regress::Regex::with_flags(pattern, f.as_str())?,
            unicode: f.contains('u'),
        })
    }

    /// First match at or after `from` (like `exec` with `lastIndex = from` on a /g regex).
    pub fn exec_at(&self, s: &JsStr, from: usize) -> Option<Caps> {
        if from > s.len() {
            return None;
        }
        let m = if self.unicode {
            self.re.find_from_utf16(&s.0, from).next()?
        } else {
            self.re.find_from_ucs2(&s.0, from).next()?
        };
        let r = m.range();
        let groups = m.captures.iter().map(|g| g.as_ref().map(|r| (r.start, r.end))).collect();
        Some(Caps {
            index: r.start,
            end: r.end,
            groups,
        })
    }

    /// `re.exec(s)` (non-global).
    pub fn exec(&self, s: &JsStr) -> Option<Caps> {
        self.exec_at(s, 0)
    }

    /// `re.test(s)` (non-global).
    pub fn test(&self, s: &JsStr) -> bool {
        self.exec(s).is_some()
    }

    /// `re.test(s)` on a Rust string.
    pub fn test_str(&self, s: &str) -> bool {
        self.test(&JsStr::new(s))
    }

    /// `re.exec(s)` on a Rust string; groups as Rust strings (`None` = undefined).
    pub fn exec_str(&self, s: &str) -> Option<Vec<Option<String>>> {
        let js = JsStr::new(s);
        let m = self.exec(&js)?;
        Some((0..=m.groups.len()).map(|i| m.str(&js, i)).collect())
    }

    /// `[...s.matchAll(re)]` / the matches a global `replace` visits.
    pub fn all(&self, s: &JsStr) -> Vec<Caps> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos <= s.len() {
            let Some(m) = self.exec_at(s, pos) else { break };
            pos = if m.end != m.index {
                m.end
            } else if self.unicode && s.at(m.end).is_some_and(|u| (0xD800..0xDC00).contains(&u)) && s.at(m.end + 1).is_some_and(|u| (0xDC00..0xE000).contains(&u)) {
                m.end + 2 // AdvanceStringIndex: a whole code point
            } else {
                m.end + 1
            };
            out.push(m);
        }
        out
    }

    /// `(s.match(re_g) ?? []).length`.
    pub fn count(&self, s: &JsStr) -> usize {
        self.all(s).len()
    }

    fn replace_impl(&self, s: &JsStr, global: bool, mut f: impl FnMut(&Caps) -> JsStr) -> JsStr {
        let matches = if global { self.all(s) } else { self.exec(s).into_iter().collect() };
        if matches.is_empty() {
            return s.clone();
        }
        let mut out = Vec::with_capacity(s.len());
        let mut last = 0;
        for m in &matches {
            out.extend_from_slice(&s.0[last..m.index]);
            out.extend_from_slice(&f(m).0);
            last = m.end;
        }
        out.extend_from_slice(&s.0[last..]);
        JsStr(out)
    }

    /// `s.replace(re, fn)` (first match).
    pub fn replace_first_with(&self, s: &JsStr, f: impl FnMut(&Caps) -> JsStr) -> JsStr {
        self.replace_impl(s, false, f)
    }

    /// `s.replace(re_g, fn)` (every match).
    pub fn replace_all_with(&self, s: &JsStr, f: impl FnMut(&Caps) -> JsStr) -> JsStr {
        self.replace_impl(s, true, f)
    }

    /// `s.replace(re, "literal")` (first match; no `$` patterns).
    pub fn replace_first(&self, s: &JsStr, rep: &str) -> JsStr {
        let r = JsStr::new(rep);
        self.replace_impl(s, false, |_| r.clone())
    }

    /// `s.replace(re_g, "literal")` (no `$` patterns).
    pub fn replace_all(&self, s: &JsStr, rep: &str) -> JsStr {
        let r = JsStr::new(rep);
        self.replace_impl(s, true, |_| r.clone())
    }

    /// `s.replace(re, "lit")` on Rust strings (first match).
    pub fn replace_first_str(&self, s: &str, rep: &str) -> String {
        self.replace_first(&JsStr::new(s), rep).to_string()
    }

    /// `s.replace(re_g, "lit")` on Rust strings.
    pub fn replace_all_str(&self, s: &str, rep: &str) -> String {
        self.replace_all(&JsStr::new(s), rep).to_string()
    }

    /// `s.split(re)[0]`: the text before the first match (the whole string when none).
    pub fn split_first(&self, s: &JsStr) -> JsStr {
        // split() skips a match that ends where the current piece starts (an empty match at 0).
        let mut q = 0;
        while q < s.len() {
            let Some(m) = self.exec_at(s, q) else { break };
            if m.index >= s.len() {
                break;
            }
            if m.end == 0 {
                q = m.index + 1;
                continue;
            }
            return s.slice(0, m.index);
        }
        s.clone()
    }
}

/// `re!(pattern)` / `re!(pattern, flags)`: a `&'static JsRegex` compiled on first use.
#[macro_export]
macro_rules! re {
    ($pat:expr) => {
        $crate::re!($pat, "")
    };
    ($pat:expr, $flags:expr) => {{
        static RE: ::std::sync::LazyLock<$crate::jsre::JsRegex> = ::std::sync::LazyLock::new(|| $crate::jsre::JsRegex::new($pat, $flags));
        &*RE
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_semantics() {
        let s = JsStr::new("a\u{feff}b 🎉x");
        assert!(re!(r"a\sb").test(&s));
        assert_eq!(s.len(), 7);
        let m = re!(r"x").exec(&s).unwrap();
        assert_eq!(m.index, 6);
        assert_eq!(re!(r"(?<![\w-])b").all(&JsStr::new("ab -b b")).len(), 1);
        assert!(re!(r"select", "i").test_str("SELECT"));
        assert!(!re!(r"k", "i").test_str("\u{212A}"));
        assert_eq!(re!(r"\s+").split_first(&JsStr::new(" a b")).to_string(), "");
        assert_eq!(re!(r"\s+").split_first(&JsStr::new("a b")).to_string(), "a");
        assert_eq!(re!(r"x*", "").replace_all_str("abc", "-"), "-a-b-c-");
        assert_eq!(re!(r"(['\x22])(.*?)\1").exec_str("say \"hi\" 'x'").unwrap()[2].as_deref(), Some("hi"));
    }
}
