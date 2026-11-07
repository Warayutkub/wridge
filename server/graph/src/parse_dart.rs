//! Dart/Flutter files → symbols, imports, references and routes (port of `parse-dart.mjs`), using
//! the official Dart analyzer through the same program the Node server drives:
//! `legacy-node/codegraph/dart/bin/parse.dart`, compiled once to `dart/build/wara-dart-parse(.exe)`.
//! It reads `{"files": [abs paths]}` on stdin and prints one JSON object on stdout.
//!
//! The folder is [`dart_dir`]: `WRIDGE_DART_DIR` when set, else `legacy-node/codegraph/dart` next
//! to this crate (override with [`set_dart_dir`]).

use crate::jsre::JsStr;
use crate::parse_ts::{hash, DeclaredRoute, Import, Parsed, Ref, Symbol};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const BATCH: usize = 400;

static DART_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Uses another folder holding `bin/parse.dart`, `pubspec.yaml` and `build/` (call before the first parse).
pub fn set_dart_dir(dir: impl Into<PathBuf>) {
    let _ = DART_DIR.set(dir.into());
}

/// The Dart parser folder.
pub fn dart_dir() -> &'static Path {
    DART_DIR.get_or_init(|| match std::env::var_os("WRIDGE_DART_DIR") {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("legacy-node")
            .join("codegraph")
            .join("dart"),
    })
}

fn exe() -> PathBuf {
    dart_dir().join("build").join(if cfg!(windows) { "wara-dart-parse.exe" } else { "wara-dart-parse" })
}

fn hide(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

struct RunOut {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// `spawnSync(cmd, args, { cwd: DIR, shell: win32 })`.
fn run(args: &[&str]) -> RunOut {
    let line = std::iter::once("dart").chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
    let mut cmd;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd = Command::new(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()));
        cmd.raw_arg(format!("/d /s /c \"{line}\""));
    }
    #[cfg(not(windows))]
    {
        cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(&line);
    }
    match hide(&mut cmd).current_dir(dart_dir()).stdin(Stdio::null()).output() {
        Ok(o) => RunOut {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => RunOut {
            ok: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

fn last_line(r: &RunOut) -> String {
    let s = if r.stderr.is_empty() { &r.stdout } else { &r.stderr };
    crate::js::js_trim(s).split('\n').next_back().unwrap_or("").to_string()
}

fn mtime(p: &Path) -> Option<f64> {
    std::fs::metadata(p).ok().map(|m| crate::js::mtime_ms(&m))
}

/// Builds the parser if it is missing or older than its source (needs the Dart SDK). Returns
/// whether it was (re)built.
pub fn ensure_parser() -> Result<bool> {
    let built = mtime(&exe()).unwrap_or(0.0);
    let dir = dart_dir();
    let sources = [dir.join("bin").join("parse.dart"), dir.join("pubspec.yaml"), dir.join("pubspec.lock")];
    if built != 0.0 && sources.iter().all(|f| mtime(f).is_none_or(|m| m <= built)) {
        return Ok(false);
    }
    if !run(&["--version"]).ok {
        bail!("Dart SDK not found — install Flutter or Dart (and put it on PATH) to graph .dart files");
    }
    let get = run(&["pub", "get"]);
    if !get.ok {
        bail!("dart pub get failed: {}", last_line(&get));
    }
    std::fs::create_dir_all(exe().parent().unwrap())?;
    let exe = exe().to_string_lossy().into_owned();
    let compile = run(&["compile", "exe", "bin/parse.dart", "-o", &exe]);
    if !compile.ok {
        bail!("Compiling the Dart parser failed: {}", last_line(&compile));
    }
    Ok(true)
}

/// Per file: the parse result, or the parser's error message.
pub type DartResult = std::result::Result<Parsed, String>;

/// `{ results, built }`.
#[derive(Debug, Default)]
pub struct DartOutput {
    pub results: HashMap<String, DartResult>,
    pub built: bool,
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        other => crate::data::js_string(other),
    }
}

fn opt_s(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        None | Some(Value::Null) => None,
        Some(Value::String(x)) => Some(x.clone()),
        other => Some(crate::data::js_string(other)),
    }
}

fn int(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64))).unwrap_or(0)
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn convert(res: &Value, source: &JsStr) -> Parsed {
    let symbols = arr(res, "symbols")
        .iter()
        .map(|x| {
            let start = x.get("start").and_then(Value::as_i64).map(|v| v.max(0) as usize).unwrap_or(0);
            let end = x.get("end").and_then(Value::as_i64).map(|v| v.max(0) as usize).unwrap_or(source.len());
            Symbol {
                name: s(x, "name"),
                kind: s(x, "kind"),
                exported: int(x, "exported"),
                is_default: int(x, "isDefault"),
                line: int(x, "line"),
                end_line: int(x, "endLine"),
                props: x.get("props").filter(|p| crate::js::truthy(Some(p))).cloned(),
                hash: hash(&source.slice(start, end).to_string()),
            }
        })
        .collect();
    let imports = arr(res, "imports")
        .iter()
        .map(|x| Import {
            kind: s(x, "kind"),
            source: s(x, "source"),
            imported: opt_s(x, "imported"),
            local: opt_s(x, "local"),
            type_only: 0,
            line: int(x, "line"),
        })
        .collect();
    let refs = arr(res, "refs")
        .iter()
        .map(|x| Ref {
            owner: x.get("owner").and_then(Value::as_i64).unwrap_or(-1),
            local: s(x, "local"),
            member: opt_s(x, "member"),
            kind: s(x, "kind"),
            line: int(x, "line"),
            count: int(x, "count"),
        })
        .collect();
    let routes = arr(res, "routes")
        .iter()
        .map(|x| DeclaredRoute {
            path: s(x, "path"),
            widget: opt_s(x, "widget"),
            kind: s(x, "kind"),
            line: int(x, "line"),
        })
        .collect();
    Parsed {
        directive: None,
        symbols,
        imports,
        refs,
        routes,
    }
}

/// Relative paths → per-file results (`parseDartFiles`). Errors when the parser cannot be built or
/// fails as a whole; per-file analyzer errors are in the results.
pub fn parse_dart_files(root: &Path, rel_paths: &[String]) -> Result<DartOutput> {
    if rel_paths.is_empty() {
        return Ok(DartOutput::default());
    }
    let built = ensure_parser()?;
    let mut results = HashMap::new();
    for chunk in rel_paths.chunks(BATCH) {
        let abs: Vec<String> = chunk.iter().map(|p| crate::paths::resolve(&root.join(p).to_string_lossy()).replace('\\', "/")).collect();
        let input = crate::js::json_stringify(&serde_json::json!({ "files": abs }));
        let child = hide(Command::new(exe()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())).spawn();
        let out = match child {
            Ok(mut c) => {
                if let Some(mut stdin) = c.stdin.take() {
                    let _ = stdin.write_all(input.as_bytes());
                }
                c.wait_with_output()?
            }
            Err(_) => bail!("Dart parser failed: "),
        };
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("Dart parser failed: {}", crate::js::js_trim(&err).split('\n').next_back().unwrap_or(""));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let parsed: Value = crate::js::json_parse(&stdout).map_err(|e| anyhow!("{e}"))?;
        for (rel, a) in chunk.iter().zip(&abs) {
            let res = parsed.get(a);
            match res {
                Some(r) if crate::js::truthy(Some(r)) && !crate::js::truthy(r.get("error")) => {
                    let source = JsStr::new(&crate::js::read_utf8(root.join(rel))?);
                    results.insert(rel.clone(), Ok(convert(r, &source)));
                }
                _ => {
                    let err = res
                        .and_then(|r| r.get("error"))
                        .filter(|e| !e.is_null())
                        .map(|e| crate::data::js_string(Some(e)))
                        .unwrap_or_else(|| "no result".into());
                    results.insert(rel.clone(), Err(err));
                }
            }
        }
    }
    Ok(DartOutput { results, built })
}
