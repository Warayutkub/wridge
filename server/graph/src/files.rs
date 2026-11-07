//! Which files belong to the graph, and keeping `.wara/` out of git (port of `files.mjs`).

use crate::paths;
use crate::re;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Stdio};

/// The folder (inside a project) holding the graph database.
pub const WARA_DIR: &str = ".wara";

const SKIP_DIRS: [&str; 19] = [
    "node_modules",
    ".git",
    ".next",
    ".turbo",
    ".vercel",
    ".dart_tool",
    "build",
    "dist",
    "out",
    "coverage",
    ".wara",
    "Pods",
    ".gradle",
    ".idea",
    "vendor",
    ".gomodcache",
    "target",
    "__pycache__",
    ".venv",
];
const MAX_BYTES: u64 = 1_000_000; // bigger files are generated/minified; parsing them only adds noise

/// Other source languages are kept at file level so the graph has no blind spots.
const OTHER: [&str; 32] = [
    ".py", ".go", ".rs", ".java", ".kt", ".kts", ".swift", ".m", ".mm", ".c", ".h", ".cc", ".cpp", ".hpp", ".cs", ".rb", ".php", ".sql", ".prisma", ".graphql", ".gql", ".proto",
    ".css", ".scss", ".sass", ".less", ".html", ".vue", ".svelte", ".astro", ".sh", ".ps1",
];

fn is_config(name: &str) -> bool {
    name == "package.json" || name == "pubspec.yaml" || re!(r"^[tj]sconfig(\..+)?\.json$").test_str(name)
}

fn code_lang(ext: &str) -> Option<&'static str> {
    Some(match ext {
        ".ts" | ".tsx" | ".js" | ".jsx" | ".mjs" | ".cjs" | ".mts" | ".cts" => "ts",
        ".dart" => "dart",
        _ => return None,
    })
}

/// `ts` | `dart` | `config` | the extension without the dot (other source files) | `None` (not in the graph).
pub fn classify(path: &str) -> Option<String> {
    let name = paths::basename(path);
    if is_config(name) {
        return Some("config".into());
    }
    if re!(r"\.(min|bundle)\.[cm]?js$").test_str(name) || re!(r"\.(g|freezed|mocks)\.dart$").test_str(name) {
        return None; // generated
    }
    let ext = paths::extname(name).to_lowercase();
    if let Some(l) = code_lang(&ext) {
        return Some(l.into());
    }
    if OTHER.contains(&ext.as_str()) {
        return Some(ext[1..].to_string());
    }
    None
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (windowsHide)
    }
    cmd
}

/// Runs `git -C <root> <args>`; stdout when it exits 0.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = hidden(Command::new("git").arg("-C").arg(root).args(args))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

/// `git rev-parse --show-toplevel` (forward slashes), or `None` outside git.
pub fn git_top(root: &Path) -> Option<String> {
    let t = git(root, &["rev-parse", "--show-toplevel"])?;
    let t = crate::js::js_trim(&t).to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

/// One file of the graph, as `listFiles` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedFile {
    /// Relative to the project root, forward slashes.
    pub path: String,
    pub lang: String,
    /// `stat.mtimeMs`.
    pub mtime: f64,
    pub size: u64,
}

/// Project files that git would track (respects every .gitignore), or a plain walk outside git.
pub fn list_files(root: &Path) -> Vec<ListedFile> {
    let out = git(root, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"]);
    let paths: Vec<String> = match out {
        Some(o) => o.split('\0').filter(|s| !s.is_empty()).map(String::from).collect(),
        None => {
            let mut acc = Vec::new();
            walk(root, "", &mut acc);
            acc
        }
    };
    let mut files = Vec::new();
    for path in paths {
        let Some(lang) = classify(&path) else { continue };
        if path.split('/').any(|seg| SKIP_DIRS.contains(&seg)) {
            continue;
        }
        let Ok(st) = std::fs::metadata(root.join(&path)) else { continue }; // deleted but still in the git index
        if !st.is_file() || st.len() > MAX_BYTES {
            continue;
        }
        files.push(ListedFile {
            mtime: crate::js::mtime_ms(&st),
            size: st.len(),
            path,
            lang,
        });
    }
    files
}

fn walk(root: &Path, dir: &str, acc: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(root.join(dir)) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != ".github" {
            continue;
        }
        let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(root, &rel, acc);
            }
        } else {
            acc.push(rel);
        }
    }
}

/// Must run before `.wara/` is created: the graph is generated, machine-specific data that never
/// belongs in git. Adds `.wara/` to the repo's root .gitignore when git does not ignore it yet.
/// Outside git it does nothing. Returns the same JSON as the Node version.
pub fn ensure_gitignored(root: &Path) -> anyhow::Result<Value> {
    let Some(top) = git_top(root) else { return Ok(json!({ "git": false })) };
    let probe = root.join(WARA_DIR).join("code-graph.db");
    let check = hidden(Command::new("git").arg("-C").arg(root).args(["check-ignore", "-q"]).arg(&probe))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if matches!(check, Ok(s) if s.success()) {
        return Ok(json!({ "git": true, "gitignore": null, "added": false }));
    }
    let file = std::path::PathBuf::from(paths::resolve(&format!("{top}/.gitignore")));
    let rel = paths::relative(&top, &root.to_string_lossy()).replace('\\', "/");
    let pattern = if rel.is_empty() { format!("/{WARA_DIR}/") } else { format!("/{rel}/{WARA_DIR}/") };
    let block = format!("\n# wridge code graph (generated, rebuilt from source)\n{pattern}\n");
    if file.exists() {
        let current = crate::js::read_utf8(&file)?;
        let sep = if current.ends_with('\n') || current.is_empty() { "" } else { "\n" };
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&file)?;
        f.write_all(format!("{sep}{block}").as_bytes())?;
    } else {
        std::fs::write(&file, crate::js::js_trim_start(&block))?;
    }
    Ok(json!({ "git": true, "gitignore": file.to_string_lossy().replace('\\', "/"), "added": true, "pattern": pattern }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_files() {
        assert_eq!(classify("src/a.tsx").as_deref(), Some("ts"));
        assert_eq!(classify("tsconfig.base.json").as_deref(), Some("config"));
        assert_eq!(classify("a/package.json").as_deref(), Some("config"));
        assert_eq!(classify("lib/x.g.dart"), None);
        assert_eq!(classify("public/app.min.js"), None);
        assert_eq!(classify("db/schema.SQL").as_deref(), Some("sql"));
        assert_eq!(classify("README.md"), None);
        assert_eq!(classify(".gitignore"), None);
    }
}
