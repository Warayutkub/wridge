//! `node:path` helpers with Node's exact results: `posix.normalize/join/dirname`, `basename`,
//! `extname`, and lexical win32 `resolve`/`relative` (Node never touches the disk for these).

fn is_sep_posix(c: u16) -> bool {
    c == b'/' as u16
}

fn is_sep_win(c: u16) -> bool {
    c == b'/' as u16 || c == b'\\' as u16
}

/// Node's internal `normalizeString`.
fn normalize_string(path: &[u16], allow_above_root: bool, sep: u16, is_sep: fn(u16) -> bool) -> Vec<u16> {
    let dot = b'.' as u16;
    let mut res: Vec<u16> = Vec::new();
    let mut last_segment_length: isize = 0;
    let mut last_slash: isize = -1;
    let mut dots: isize = 0;
    let mut code: u16 = 0;
    let n = path.len();
    let mut i = 0usize;
    while i <= n {
        if i < n {
            code = path[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/' as u16;
        }
        if is_sep(code) {
            if last_slash == i as isize - 1 || dots == 1 {
                // noop
            } else if dots == 2 {
                if res.len() < 2 || last_segment_length != 2 || res[res.len() - 1] != dot || res[res.len() - 2] != dot {
                    if res.len() > 2 {
                        match res.iter().rposition(|c| *c == sep) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(idx) => {
                                res.truncate(idx);
                                let li = res.iter().rposition(|c| *c == sep).map(|x| x as isize).unwrap_or(-1);
                                last_segment_length = res.len() as isize - 1 - li;
                            }
                        }
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(sep);
                    }
                    res.push(dot);
                    res.push(dot);
                    last_segment_length = 2;
                }
            } else {
                let seg = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(sep);
                }
                res.extend_from_slice(seg);
                last_segment_length = i as isize - last_slash - 1;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == dot && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

/// `path.posix.normalize(p)`.
pub fn posix_normalize(p: &str) -> String {
    let u: Vec<u16> = p.encode_utf16().collect();
    if u.is_empty() {
        return ".".into();
    }
    let is_abs = u[0] == b'/' as u16;
    let trailing = *u.last().unwrap() == b'/' as u16;
    let mut out = normalize_string(&u, !is_abs, b'/' as u16, is_sep_posix);
    if out.is_empty() {
        if is_abs {
            return "/".into();
        }
        return if trailing { "./".into() } else { ".".into() };
    }
    if trailing {
        out.push(b'/' as u16);
    }
    let s = String::from_utf16_lossy(&out);
    if is_abs {
        format!("/{s}")
    } else {
        s
    }
}

/// `path.posix.join(...parts)`.
pub fn posix_join(parts: &[&str]) -> String {
    let kept: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if kept.is_empty() {
        return ".".into();
    }
    posix_normalize(&kept.join("/"))
}

/// `path.posix.dirname(p)`.
pub fn posix_dirname(p: &str) -> String {
    let u: Vec<u16> = p.encode_utf16().collect();
    if u.is_empty() {
        return ".".into();
    }
    let slash = b'/' as u16;
    let has_root = u[0] == slash;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = u.len() as isize - 1;
    while i >= 1 {
        if u[i as usize] == slash {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }
    if end == -1 {
        return if has_root { "/".into() } else { ".".into() };
    }
    if has_root && end == 1 {
        return "//".into();
    }
    String::from_utf16_lossy(&u[..end as usize])
}

/// `posix.dirname(p) === "." ? "" : posix.dirname(p)` — the folder key used across the graph.
pub fn dir_key(p: &str) -> String {
    let d = posix_dirname(p);
    if d == "." {
        String::new()
    } else {
        d
    }
}

/// `path.basename(p)` (win32: `/` and `\` both separate).
pub fn basename(p: &str) -> &str {
    let t = p.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

/// `path.extname(p)`: from the last `.` of the last segment; `""` for dotfiles like `.gitignore`.
pub fn extname(p: &str) -> String {
    let u: Vec<u16> = p.encode_utf16().collect();
    let mut start_dot: isize = -1;
    let mut start_part: isize = 0;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0;
    let mut i = u.len() as isize - 1;
    while i >= 0 {
        let code = u[i as usize];
        if is_sep_win(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if code == b'.' as u16 {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }
    if start_dot == -1 || end == -1 || pre_dot_state == 0 || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1) {
        return String::new();
    }
    String::from_utf16_lossy(&u[start_dot as usize..end as usize])
}

/// `path.isAbsolute(p)` on Windows (`C:\x`, `C:/x`, `\x`, `/x`, `\\server`).
pub fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    if b.is_empty() {
        return false;
    }
    if b[0] == b'/' || b[0] == b'\\' {
        return true;
    }
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
}

/// Lexical `path.resolve(p)` (win32 on Windows): absolute, `\` separators, no `.`/`..`, no
/// trailing separator except at a drive root.
pub fn resolve(p: &str) -> String {
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    if !cfg!(windows) {
        let joined = if p.starts_with('/') { p.to_string() } else { format!("{cwd}/{p}") };
        let n = posix_normalize(&joined);
        return if n.len() > 1 { n.trim_end_matches('/').to_string() } else { n };
    }
    let unify = |s: &str| s.replace('/', "\\");
    let p = unify(p);
    let cwd = unify(&cwd);
    let (device, rest) = split_device(&p);
    let full = if (!device.is_empty() && rest.starts_with('\\')) || device.starts_with("\\\\") {
        p.clone()
    } else if rest.starts_with('\\') {
        let (cd, _) = split_device(&cwd);
        format!("{cd}{rest}")
    } else if !device.is_empty() {
        // drive-relative ("C:x"): relative to that drive's cwd; use the cwd when it is on that drive
        let (cd, crest) = split_device(&cwd);
        if cd.eq_ignore_ascii_case(device) {
            format!("{cd}{crest}\\{rest}")
        } else {
            format!("{device}\\{rest}")
        }
    } else {
        format!("{cwd}\\{p}")
    };
    let (device, rest) = split_device(&full);
    let units: Vec<u16> = rest.encode_utf16().collect();
    let norm = String::from_utf16_lossy(&normalize_string(&units, false, b'\\' as u16, is_sep_win));
    format!("{device}\\{norm}")
}

fn split_device(p: &str) -> (&str, &str) {
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return (&p[..2], &p[2..]);
    }
    if let Some(unc) = p.strip_prefix("\\\\") {
        // \\server\share
        let parts: Vec<&str> = unc.splitn(3, '\\').collect();
        if parts.len() >= 2 {
            let len = 2 + parts[0].len() + 1 + parts[1].len();
            return (&p[..len], &p[len..]);
        }
    }
    ("", p)
}

/// `path.relative(from, to)` (win32: case-insensitive, result uses `\`).
pub fn relative(from: &str, to: &str) -> String {
    let f = resolve(from);
    let t = resolve(to);
    if f == t {
        return String::new();
    }
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let fl: Vec<String> = f
        .split(sep)
        .filter(|s| !s.is_empty())
        .map(|s| if cfg!(windows) { s.to_lowercase() } else { s.to_string() })
        .collect();
    let tparts: Vec<&str> = t.split(sep).filter(|s| !s.is_empty()).collect();
    let tl: Vec<String> = tparts.iter().map(|s| if cfg!(windows) { s.to_lowercase() } else { s.to_string() }).collect();
    if cfg!(windows) && fl.first() != tl.first() {
        return t;
    }
    let mut common = 0;
    while common < fl.len() && common < tl.len() && fl[common] == tl[common] {
        common += 1;
    }
    let mut out: Vec<String> = vec!["..".to_string(); fl.len() - common];
    out.extend(tparts[common..].iter().map(|s| s.to_string()));
    out.join(&sep.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix() {
        assert_eq!(posix_join(&["src", "../lib/x"]), "lib/x");
        assert_eq!(posix_join(&["", "a"]), "a");
        assert_eq!(posix_join(&["a", "./b/"]), "a/b/");
        assert_eq!(posix_normalize("./a//b/../c"), "a/c");
        assert_eq!(posix_normalize("../../x"), "../../x");
        assert_eq!(posix_dirname("a/b/c.ts"), "a/b");
        assert_eq!(posix_dirname("c.ts"), ".");
        assert_eq!(extname("a.d.ts"), ".ts");
        assert_eq!(extname(".gitignore"), "");
        assert_eq!(extname("..foo"), ".foo");
        assert_eq!(basename("a/b\\c.ts"), "c.ts");
    }
}
