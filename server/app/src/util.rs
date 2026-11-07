//! Small helpers shared by the server modules: the log line, Node-style file errors, JS string → number.

use std::path::{Component, Path, PathBuf};

/// `log(...)` in index.mjs: `[HH:MM:SS] message` on stdout (the wridge app shows these lines).
pub fn log(message: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), message);
    let _ = out.flush();
}

/// The message Node's fs functions throw, e.g. `ENOENT: no such file or directory, open 'C:/x.png'`.
pub fn node_fs_error(err: &std::io::Error, syscall: &str, path: &str) -> anyhow::Error {
    use std::io::ErrorKind::*;
    let msg = match err.kind() {
        NotFound => format!("ENOENT: no such file or directory, {syscall} '{path}'"),
        PermissionDenied => format!("EACCES: permission denied, {syscall} '{path}'"),
        IsADirectory => "EISDIR: illegal operation on a directory, read".to_string(),
        _ => match err.raw_os_error() {
            // Windows reports a directory opened as a file as "access denied"
            Some(5) if Path::new(path).is_dir() => "EISDIR: illegal operation on a directory, read".to_string(),
            _ => format!("{err}, {syscall} '{path}'"),
        },
    };
    anyhow::anyhow!(msg)
}

/// The path as Node shows it in fs errors: resolved against the working folder on Windows.
pub fn error_path(path: &Path) -> String {
    if cfg!(windows) {
        path_resolve(Path::new(""), &path.display().to_string()).display().to_string()
    } else {
        path.display().to_string()
    }
}

/// `readFileSync(path)` with Node's error text.
pub fn read_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| node_fs_error(&e, "open", &error_path(path)))
}

/// `Number(string)`.
pub fn js_number(s: &str) -> f64 {
    let t = crate::jsjson::js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |p: &str, r: u32| u64::from_str_radix(p, r).map(|v| v as f64).unwrap_or(f64::NAN);
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return radix(h, 16);
    }
    if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return radix(o, 8);
    }
    if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return radix(b, 2);
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let ok = t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if !ok || t.contains("inf") || t.contains("nan") {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// Node's `path.resolve(...)` for one or two segments, on this platform's rules (backslashes on Windows).
pub fn path_resolve(base: &Path, p: &str) -> PathBuf {
    let joined = if Path::new(p).is_absolute() { PathBuf::from(p) } else { base.join(p) };
    let abs = if joined.is_absolute() {
        joined
    } else {
        std::env::current_dir().unwrap_or_default().join(joined)
    };
    normalize(&abs)
}

/// Collapse `.` and `..` without touching the disk.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Forward slashes, like `.replaceAll("\\", "/")` on a Windows path.
pub fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Lenient base64 like `Buffer.from(s, "base64")`: ignores characters outside the alphabet, padding optional.
pub fn base64_lenient(s: &str) -> Vec<u8> {
    use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
    use base64::engine::DecodePaddingMode;
    use base64::Engine;
    let clean: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_'))
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    let clean = if clean.len() % 4 == 1 { &clean[..clean.len() - 1] } else { &clean[..] };
    let engine = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(clean).unwrap_or_default()
}

/// codegen.mjs `writeFileDeep(file, data)`: `mkdir(dirname(file), { recursive: true })`, then the file.
/// (wridge-figma's `codegen::write_file_deep` takes text; assets and PDFs are bytes.)
pub fn write_file_deep(file: &str, data: &[u8]) -> anyhow::Result<()> {
    if let Some(dir) = Path::new(file).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| node_fs_error(&e, "mkdir", &dir.display().to_string()))?;
    }
    std::fs::write(file, data).map_err(|e| node_fs_error(&e, "open", file))
}

/// `encodeURIComponent(s)`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `(n / 1024).toFixed(1)` for a byte count: exact, halves round up like toFixed.
pub fn kilobytes(n: usize) -> String {
    let tenths = (n as u128 * 20 + 1024) / 2048;
    format!("{}.{}", tenths / 10, tenths % 10)
}

pub fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command that never flashes a console window (Node's `windowsHide: true`).
pub fn hidden_command(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_from_string_like_js() {
        assert_eq!(js_number(" 4 "), 4.0);
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert!(js_number("abc").is_nan());
        assert_eq!(js_number("2.5"), 2.5);
    }

    #[test]
    fn lenient_base64() {
        assert_eq!(base64_lenient("aGVsbG8="), b"hello");
        assert_eq!(base64_lenient("aGVsbG8"), b"hello");
        assert_eq!(base64_lenient("aGVs\nbG8="), b"hello");
    }

    #[test]
    fn kilobytes_like_to_fixed() {
        // (n / 1024).toFixed(1) in Node
        for (n, s) in [
            (0, "0.0"),
            (51, "0.0"),
            (52, "0.1"),
            (256, "0.3"),
            (1075, "1.0"),
            (1126, "1.1"),
            (2867, "2.8"),
            (10_239_999, "10000.0"),
        ] {
            assert_eq!(kilobytes(n), s, "{n}");
        }
    }

    #[test]
    fn uri_component_like_js() {
        assert_eq!(
            encode_uri_component("Book ~!*()' ไทย 😀.pdf"),
            "Book%20~!*()'%20%E0%B9%84%E0%B8%97%E0%B8%A2%20%F0%9F%98%80.pdf"
        );
        assert_eq!(encode_uri_component("a+b/c?d=e&f#g"), "a%2Bb%2Fc%3Fd%3De%26f%23g");
    }

    #[test]
    fn write_file_deep_makes_folders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a").join("b").join("c.bin");
        write_file_deep(&file.display().to_string(), &[1, 2, 3]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), vec![1, 2, 3]);
    }
}
