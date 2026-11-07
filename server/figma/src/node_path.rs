//! Node's `path` module (`resolve`, `relative`, `join`, `normalize`, `isAbsolute`), ported line by line from
//! lib/path.js so paths, error texts and "inside the project" checks come out as the Node server's do.
//! The platform flavour is picked like Node does: win32 on Windows, posix elsewhere.

#[cfg(windows)]
pub use win32::*;
#[cfg(not(windows))]
pub use posix::*;

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn string(v: &[u16]) -> String {
    String::from_utf16_lossy(v)
}
fn lower(v: &[u16]) -> Vec<u16> {
    u16s(&string(v).to_lowercase())
}

fn cwd() -> String {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| ".".into())
}

// Resolves . and .. elements in a path with directory names
fn normalize_string(path: &[u16], allow_above_root: bool, separator: u16, is_sep: fn(u16) -> bool) -> Vec<u16> {
    let mut res: Vec<u16> = vec![];
    let mut last_segment_length: i64 = 0;
    let mut last_slash: i64 = -1;
    let mut dots: i64 = 0;
    let mut code: u16 = 0;
    let len = path.len() as i64;
    let mut i: i64 = 0;
    while i <= len {
        if i < len {
            code = path[i as usize];
        } else if is_sep(code) {
            break;
        } else {
            code = SLASH;
        }
        if is_sep(code) {
            if last_slash == i - 1 || dots == 1 {
                // NOOP
            } else if dots == 2 {
                if res.len() < 2 || last_segment_length != 2 || res[res.len() - 1] != DOT || res[res.len() - 2] != DOT {
                    if res.len() > 2 {
                        match res.iter().rposition(|c| *c == separator) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(idx) => {
                                res.truncate(idx);
                                last_segment_length = res.len() as i64 - 1 - res.iter().rposition(|c| *c == separator).map_or(-1, |p| p as i64);
                            }
                        }
                        last_slash = i;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(separator);
                    }
                    res.extend([DOT, DOT]);
                    last_segment_length = 2;
                }
            } else {
                let seg = &path[(last_slash + 1) as usize..i as usize];
                if !res.is_empty() {
                    res.push(separator);
                }
                res.extend_from_slice(seg);
                last_segment_length = i - last_slash - 1;
            }
            last_slash = i;
            dots = 0;
        } else if code == DOT && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

pub mod win32 {
    use super::*;

    fn is_sep(c: u16) -> bool {
        c == SLASH || c == BACKSLASH
    }
    fn is_device_root(c: u16) -> bool {
        (b'A' as u16..=b'Z' as u16).contains(&c) || (b'a' as u16..=b'z' as u16).contains(&c)
    }

    /// `path.win32.resolve(...paths)`.
    pub fn resolve(args: &[&str]) -> String {
        let mut resolved_device: Vec<u16> = vec![];
        let mut resolved_tail: Vec<u16> = vec![];
        let mut resolved_absolute = false;
        let mut i = args.len() as i64 - 1;
        while i >= -1 {
            let path: Vec<u16> = if i >= 0 {
                let p = u16s(args[i as usize]);
                if p.is_empty() {
                    i -= 1;
                    continue;
                }
                p
            } else if resolved_device.is_empty() {
                let p = u16s(&cwd());
                if args.is_empty() || (args.len() == 1 && (args[0].is_empty() || args[0] == ".") && p.first().is_some_and(|c| is_sep(*c))) {
                    return string(&p);
                }
                p
            } else {
                let env = std::env::var(format!("={}", string(&resolved_device))).ok().filter(|s| !s.is_empty()).unwrap_or_else(cwd);
                let p = u16s(&env);
                if lower(&p[..p.len().min(2)]) != lower(&resolved_device) && p.get(2) == Some(&BACKSLASH) {
                    let mut d = resolved_device.clone();
                    d.push(BACKSLASH);
                    d
                } else {
                    p
                }
            };
            let len = path.len();
            let mut root_end = 0;
            let mut device: Vec<u16> = vec![];
            let mut is_absolute = false;
            let code = path[0];
            if len == 1 {
                if is_sep(code) {
                    root_end = 1;
                    is_absolute = true;
                }
            } else if is_sep(code) {
                // Possible UNC root
                is_absolute = true;
                if is_sep(path[1]) {
                    let mut j = 2;
                    let mut last = j;
                    while j < len && !is_sep(path[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        let first_part = path[last..j].to_vec();
                        last = j;
                        while j < len && is_sep(path[j]) {
                            j += 1;
                        }
                        if j < len && j != last {
                            last = j;
                            while j < len && !is_sep(path[j]) {
                                j += 1;
                            }
                            if j == len || j != last {
                                if first_part != [DOT] && first_part != [b'?' as u16] {
                                    device = u16s("\\\\");
                                    device.extend(&first_part);
                                    device.push(BACKSLASH);
                                    device.extend(&path[last..j]);
                                    root_end = j;
                                } else {
                                    device = u16s("\\\\");
                                    device.extend(&first_part);
                                    root_end = 4;
                                }
                            }
                        }
                    }
                } else {
                    root_end = 1;
                }
            } else if is_device_root(code) && path[1] == COLON {
                device = path[..2].to_vec();
                root_end = 2;
                if len > 2 && is_sep(path[2]) {
                    is_absolute = true;
                    root_end = 3;
                }
            }
            if !device.is_empty() {
                if !resolved_device.is_empty() {
                    if lower(&device) != lower(&resolved_device) {
                        i -= 1;
                        continue; // another device
                    }
                } else {
                    resolved_device = device;
                }
            }
            if resolved_absolute {
                if !resolved_device.is_empty() {
                    break;
                }
            } else {
                let mut t = path[root_end.min(len)..].to_vec();
                t.push(BACKSLASH);
                t.extend(&resolved_tail);
                resolved_tail = t;
                resolved_absolute = is_absolute;
                if is_absolute && !resolved_device.is_empty() {
                    break;
                }
            }
            i -= 1;
        }
        let tail = normalize_string(&resolved_tail, !resolved_absolute, BACKSLASH, is_sep);
        if resolved_absolute {
            format!("{}\\{}", string(&resolved_device), string(&tail))
        } else {
            let s = format!("{}{}", string(&resolved_device), string(&tail));
            if s.is_empty() {
                ".".into()
            } else {
                s
            }
        }
    }

    /// `path.win32.normalize(path)`.
    pub fn normalize(p: &str) -> String {
        let path = u16s(p);
        let len = path.len();
        if len == 0 {
            return ".".into();
        }
        let mut root_end = 0;
        let mut device: Option<Vec<u16>> = None;
        let mut is_absolute = false;
        let code = path[0];
        if len == 1 {
            return if code == SLASH { "\\".into() } else { p.into() };
        }
        if is_sep(code) {
            is_absolute = true;
            if is_sep(path[1]) {
                let mut j = 2;
                let mut last = j;
                while j < len && !is_sep(path[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    let first_part = path[last..j].to_vec();
                    last = j;
                    while j < len && is_sep(path[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !is_sep(path[j]) {
                            j += 1;
                        }
                        if j == len {
                            return format!("\\\\{}\\{}\\", string(&first_part), string(&path[last..]));
                        }
                        if j != last {
                            let mut d = u16s("\\\\");
                            d.extend(&first_part);
                            d.push(BACKSLASH);
                            d.extend(&path[last..j]);
                            device = Some(d);
                            root_end = j;
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if is_device_root(code) && path[1] == COLON {
            device = Some(path[..2].to_vec());
            root_end = 2;
            if len > 2 && is_sep(path[2]) {
                is_absolute = true;
                root_end = 3;
            }
        }
        let mut tail = if root_end < len { normalize_string(&path[root_end..], !is_absolute, BACKSLASH, is_sep) } else { vec![] };
        if tail.is_empty() && !is_absolute {
            tail = vec![DOT];
        }
        if !tail.is_empty() && is_sep(path[len - 1]) {
            tail.push(BACKSLASH);
        }
        if !is_absolute && device.is_none() && path.contains(&COLON) {
            // A relative path must not turn into something Windows reads as absolute ("C:…", "x:\…").
            if tail.len() >= 2 && is_device_root(tail[0]) && tail[1] == COLON {
                return format!(".\\{}", string(&tail));
            }
            for (i, c) in path.iter().enumerate() {
                if *c == COLON && (i == len - 1 || is_sep(path[i + 1])) {
                    return format!(".\\{}", string(&tail));
                }
            }
        }
        match device {
            None => {
                if is_absolute {
                    format!("\\{}", string(&tail))
                } else {
                    string(&tail)
                }
            }
            Some(d) => {
                if is_absolute {
                    format!("{}\\{}", string(&d), string(&tail))
                } else {
                    format!("{}{}", string(&d), string(&tail))
                }
            }
        }
    }

    /// `path.win32.join(...paths)`.
    pub fn join(args: &[&str]) -> String {
        let parts: Vec<&str> = args.iter().copied().filter(|a| !a.is_empty()).collect();
        if parts.is_empty() {
            return ".".into();
        }
        let first = u16s(parts[0]);
        let mut joined = u16s(&parts.join("\\"));
        let mut needs_replace = true;
        let mut slash_count = 0;
        if is_sep(first[0]) {
            slash_count += 1;
            if first.len() > 1 && is_sep(first[1]) {
                slash_count += 1;
                if first.len() > 2 {
                    if is_sep(first[2]) {
                        slash_count += 1;
                    } else {
                        needs_replace = false;
                    }
                }
            }
        }
        if needs_replace {
            while slash_count < joined.len() && is_sep(joined[slash_count]) {
                slash_count += 1;
            }
            if slash_count >= 2 {
                let mut j = vec![BACKSLASH];
                j.extend(&joined[slash_count..]);
                joined = j;
            }
        }
        normalize(&string(&joined))
    }

    /// `path.win32.isAbsolute(path)`.
    pub fn is_absolute(p: &str) -> bool {
        let path = u16s(p);
        let Some(&code) = path.first() else { return false };
        is_sep(code) || (path.len() > 2 && is_device_root(code) && path[1] == COLON && is_sep(path[2]))
    }

    /// `path.win32.relative(from, to)`.
    pub fn relative(from: &str, to: &str) -> String {
        if from == to {
            return String::new();
        }
        let from_orig = u16s(&resolve(&[from]));
        let to_orig = u16s(&resolve(&[to]));
        if from_orig == to_orig {
            return String::new();
        }
        let from = lower(&from_orig);
        let to = lower(&to_orig);
        if from == to {
            return String::new();
        }
        if from_orig.len() != from.len() || to_orig.len() != to.len() {
            let split = |v: &[u16]| -> Vec<Vec<u16>> {
                let mut parts: Vec<Vec<u16>> = v.split(|c| *c == BACKSLASH).map(|s| s.to_vec()).collect();
                if parts.last().is_some_and(|p| p.is_empty()) {
                    parts.pop();
                }
                parts
            };
            let fs = split(&from_orig);
            let ts = split(&to_orig);
            let length = fs.len().min(ts.len());
            let mut i = 0;
            while i < length {
                if lower(&fs[i]) != lower(&ts[i]) {
                    break;
                }
                i += 1;
            }
            let join_rest = |from: usize| ts[from..].iter().map(|p| string(p)).collect::<Vec<_>>().join("\\");
            if i == 0 {
                return string(&to_orig);
            } else if i == length {
                if ts.len() > length {
                    return join_rest(i);
                }
                if fs.len() > length {
                    return "..\\".repeat(fs.len() - 1 - i) + "..";
                }
                return String::new();
            }
            return "..\\".repeat(fs.len() - i) + &join_rest(i);
        }
        let mut from_start = 0;
        while from_start < from.len() && from[from_start] == BACKSLASH {
            from_start += 1;
        }
        let mut from_end = from.len();
        while from_end > from_start + 1 && from[from_end - 1] == BACKSLASH {
            from_end -= 1;
        }
        let from_len = from_end - from_start;
        let mut to_start = 0;
        while to_start < to.len() && to[to_start] == BACKSLASH {
            to_start += 1;
        }
        let mut to_end = to.len();
        while to_end > to_start + 1 && to[to_end - 1] == BACKSLASH {
            to_end -= 1;
        }
        let to_len = to_end - to_start;
        let length = from_len.min(to_len);
        let mut last_common_sep: i64 = -1;
        let mut i = 0;
        while i < length {
            let fc = from[from_start + i];
            if fc != to[to_start + i] {
                break;
            } else if fc == BACKSLASH {
                last_common_sep = i as i64;
            }
            i += 1;
        }
        if i != length {
            if last_common_sep == -1 {
                return string(&to_orig);
            }
        } else {
            if to_len > length {
                if to[to_start + i] == BACKSLASH {
                    return string(&to_orig[to_start + i + 1..]);
                }
                if i == 2 {
                    return string(&to_orig[to_start + i..]);
                }
            }
            if from_len > length {
                if from[from_start + i] == BACKSLASH {
                    last_common_sep = i as i64;
                } else if i == 2 {
                    last_common_sep = 3;
                }
            }
            if last_common_sep == -1 {
                last_common_sep = 0;
            }
        }
        let mut out = String::new();
        let mut k = from_start as i64 + last_common_sep + 1;
        while k <= from_end as i64 {
            if k == from_end as i64 || from[k as usize] == BACKSLASH {
                out.push_str(if out.is_empty() { ".." } else { "\\.." });
            }
            k += 1;
        }
        let mut to_start = to_start + last_common_sep as usize;
        if !out.is_empty() {
            return format!("{out}{}", string(&to_orig[to_start.min(to_end)..to_end]));
        }
        if to_orig.get(to_start) == Some(&BACKSLASH) {
            to_start += 1;
        }
        string(&to_orig[to_start.min(to_end)..to_end])
    }
}

pub mod posix {
    use super::*;

    fn is_sep(c: u16) -> bool {
        c == SLASH
    }

    /// `path.posix.resolve(...paths)`.
    pub fn resolve(args: &[&str]) -> String {
        let mut resolved: Vec<u16> = vec![];
        let mut absolute = false;
        let mut i = args.len() as i64 - 1;
        while i >= -1 && !absolute {
            let p = if i >= 0 { u16s(args[i as usize]) } else { u16s(&cwd()) };
            i -= 1;
            if p.is_empty() {
                continue;
            }
            let mut r = p.clone();
            r.push(SLASH);
            r.extend(&resolved);
            resolved = r;
            absolute = p[0] == SLASH;
        }
        let n = normalize_string(&resolved, !absolute, SLASH, is_sep);
        if absolute {
            format!("/{}", string(&n))
        } else if n.is_empty() {
            ".".into()
        } else {
            string(&n)
        }
    }

    /// `path.posix.normalize(path)`.
    pub fn normalize(p: &str) -> String {
        let path = u16s(p);
        if path.is_empty() {
            return ".".into();
        }
        let absolute = path[0] == SLASH;
        let trailing = path[path.len() - 1] == SLASH;
        let mut n = normalize_string(&path, !absolute, SLASH, is_sep);
        if n.is_empty() {
            if absolute {
                return "/".into();
            }
            return if trailing { "./".into() } else { ".".into() };
        }
        if trailing {
            n.push(SLASH);
        }
        if absolute {
            format!("/{}", string(&n))
        } else {
            string(&n)
        }
    }

    /// `path.posix.join(...paths)`.
    pub fn join(args: &[&str]) -> String {
        let parts: Vec<&str> = args.iter().copied().filter(|a| !a.is_empty()).collect();
        if parts.is_empty() {
            return ".".into();
        }
        normalize(&parts.join("/"))
    }

    /// `path.posix.isAbsolute(path)`.
    pub fn is_absolute(p: &str) -> bool {
        p.starts_with('/')
    }

    /// `path.posix.relative(from, to)`.
    pub fn relative(from: &str, to: &str) -> String {
        if from == to {
            return String::new();
        }
        let from = u16s(&resolve(&[from]));
        let to = u16s(&resolve(&[to]));
        if from == to {
            return String::new();
        }
        let from_start = 1;
        let from_end = from.len();
        let from_len = from_end - from_start;
        let to_start = 1;
        let to_len = to.len() - to_start;
        let length = from_len.min(to_len);
        let mut last_common_sep: i64 = -1;
        let mut i = 0;
        while i < length {
            let fc = from[from_start + i];
            if fc != to[to_start + i] {
                break;
            } else if fc == SLASH {
                last_common_sep = i as i64;
            }
            i += 1;
        }
        if i == length {
            if to_len > length {
                if to[to_start + i] == SLASH {
                    return string(&to[to_start + i + 1..]);
                }
                if i == 0 {
                    return string(&to[to_start + i..]);
                }
            } else if from_len > length {
                if from[from_start + i] == SLASH {
                    last_common_sep = i as i64;
                } else if i == 0 {
                    last_common_sep = 0;
                }
            }
        }
        let mut out = String::new();
        let mut k = from_start as i64 + last_common_sep + 1;
        while k <= from_end as i64 {
            if k == from_end as i64 || from[k as usize] == SLASH {
                out.push_str(if out.is_empty() { ".." } else { "/.." });
            }
            k += 1;
        }
        format!("{out}{}", string(&to[(to_start as i64 + last_common_sep) as usize..]))
    }
}
