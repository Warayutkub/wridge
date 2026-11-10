//! Reports: the AI sends blocks, wridge lays them out (Figma "wridge Reports · … · Approved"). A report is a file,
//! `<project>/docs/reports/<name>.json`; saved themes live in `docs/reports/themes/<name>.json`. Looks come from
//! [`DEFAULT_THEME`] unless the report, a block or an item says otherwise (the closest setting wins — the page
//! does the merging). `tasks` blocks are filled from the Tasks board when the report is read.
//!
//! Export: PNG / PDF through Edge (or Chrome) headless on the report-only page `/report-view`, so the file looks
//! exactly like the window; "copy" puts the PNG on the Windows clipboard.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail};
use serde_json::{json, Map, Value};

use crate::tasks;

pub const BLOCKS: &[&str] = &["header", "stats", "compare", "chart", "board", "table", "timeline", "callout", "checklist", "flow", "text", "image", "tasks", "divider"];

pub fn default_theme() -> Value {
    json!({
        "page": "#FBFAF8", "card": "#FFFFFF", "line": "#E2DED6", "soft": "#EBE8E2",
        "title": "#111111", "text": "#3A3833", "muted": "#77736B", "faint": "#A8A49B",
        "accent": "#2F6BFF", "ok": "#1F9D55", "warn": "#C27C0E", "bad": "#D23B2F", "info": "#2F6BFF", "neutral": "#A8A49B",
        "before": "#C9C4BA", "after": "accent", "series": ["accent", "ok", "warn", "#7A3BC4", "#0E9AA7", "bad"], "grid": "#EBE8E2",
        "radius": 14, "font": "IBM Plex Sans Thai", "mono": "IBM Plex Mono", "density": "comfortable", "width": 1440,
        "scale": 1, "sizes": {}, "weights": {}
    })
}

fn reports_dir(dir: &str) -> PathBuf {
    Path::new(dir).join("docs").join("reports")
}
fn themes_dir(dir: &str) -> PathBuf {
    reports_dir(dir).join("themes")
}

/// A safe file name: letters (any script), digits, - and _; spaces become -.
pub fn slug(name: &str) -> anyhow::Result<String> {
    let s: String = name
        .trim()
        .trim_end_matches(".json")
        .chars()
        .map(|c| if c.is_whitespace() || c == '·' || c == '/' || c == '\\' { '-' } else { c })
        .filter(|c| c.is_alphanumeric() || ('\u{0E00}'..='\u{0E7F}').contains(c) || *c == '-' || *c == '_')
        .collect::<String>()
        .to_lowercase();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() || s.starts_with('.') || s.contains("..") {
        bail!("give the report a name, e.g. \"github-phase-1\"");
    }
    Ok(s.chars().take(80).collect())
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { anyhow!("no such report: {}", path.display()) } else { e.into() })?;
    serde_json::from_str(&text).map_err(|e| anyhow!("{} is not valid JSON: {e}", path.display()))
}

fn write_json(path: &Path, v: &Value) -> anyhow::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    // the report folder (themes sit inside it) stays out of git
    if let Some(r) = path.ancestors().find(|a| a.file_name().map(|n| n == "reports").unwrap_or(false)) {
        tasks::keep_out_of_git(r)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(v)? + "\n")?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Every block of a report (rows hold one block or a list of up to 3).
fn blocks(report: &Value) -> Vec<&Value> {
    let mut out = vec![];
    for row in report.get("rows").and_then(Value::as_array).into_iter().flatten() {
        match row {
            Value::Array(a) => out.extend(a.iter()),
            b => out.push(b),
        }
    }
    out
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn list(dir: &str) -> anyhow::Result<Value> {
    tasks::check_dir(dir)?;
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(reports_dir(dir)) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") || !p.is_file() {
                continue;
            }
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            let (title, n) = match read_json(&p) {
                Ok(v) => (v.get("title").and_then(Value::as_str).unwrap_or(&name).to_string(), blocks(&v).len()),
                Err(_) => (format!("{name} (unreadable)"), 0),
            };
            out.push(json!({"name": name, "title": title, "blocks": n, "updated": modified(&p)}));
        }
    }
    out.sort_by(|a, b| b["updated"].as_u64().cmp(&a["updated"].as_u64()));
    let mut themes: Vec<String> = std::fs::read_dir(themes_dir(dir)).into_iter().flatten().flatten().filter_map(|e| {
        let p = e.path();
        (p.extension().and_then(|x| x.to_str()) == Some("json")).then(|| p.file_stem().unwrap().to_string_lossy().to_string())
    }).collect();
    themes.sort();
    Ok(json!({"reports": out, "themes": themes, "folder": reports_dir(dir).display().to_string().replace('\\', "/")}))
}

/// A short fingerprint of the reports folder (names + change times), so the page can wait for a change.
pub fn signature(dir: &str) -> String {
    let mut parts: Vec<String> = std::fs::read_dir(reports_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|e| format!("{}:{}", e.file_name().to_string_lossy(), modified(&e.path())))
        .collect();
    parts.sort();
    let mut h: u64 = 1469598103934665603;
    for b in parts.join("|").bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    format!("{}-{h:x}", parts.len())
}

/// Checks a report the AI wrote: a title, rows of known blocks; returns warnings (unknown keys are kept).
pub fn validate(report: &Value) -> anyhow::Result<Vec<String>> {
    let o = report.as_object().ok_or_else(|| anyhow!("report must be an object: {{title, rows: [...]}}"))?;
    if o.get("title").and_then(Value::as_str).map(str::trim).unwrap_or("").is_empty() {
        bail!("report needs a title");
    }
    let rows = o.get("rows").and_then(Value::as_array).ok_or_else(|| anyhow!("report needs rows: a list of blocks (or of lists of up to 3 blocks side by side)"))?;
    let mut warn = vec![];
    for (i, row) in rows.iter().enumerate() {
        let list: Vec<&Value> = match row {
            Value::Array(a) => {
                if a.is_empty() || a.len() > 3 {
                    bail!("row {} holds {} blocks — 1 to 3 side by side", i + 1, a.len());
                }
                a.iter().collect()
            }
            b => vec![b],
        };
        for b in list {
            let t = b.get("type").and_then(Value::as_str).unwrap_or("");
            if !BLOCKS.contains(&t) {
                bail!("row {}: unknown block type \"{t}\" — use {}", i + 1, BLOCKS.join(", "));
            }
            let need = |k: &str| b.get(k).map(|v| !v.is_null()).unwrap_or(false);
            let missing = match t {
                "stats" | "timeline" | "checklist" => (!need("items")).then_some("items"),
                "compare" => (!need("rows")).then_some("rows"),
                "chart" => {
                    let kind = b.get("kind").and_then(Value::as_str).unwrap_or(if need("items") && !need("series") { "hbars" } else { "line" });
                    let (ok, what) = match kind {
                        "line" | "bars" | "stacked" | "area" => (need("series"), "series"),
                        "hbars" | "donut" | "pie" | "funnel" => (need("items"), "items"),
                        "scatter" => (need("points") || need("series"), "points"),
                        "gauge" => (need("value") || need("items"), "value (or items)"),
                        "heatmap" => (need("days") || need("values"), "days (or values)"),
                        k => bail!("row {}: unknown chart kind \"{k}\" — line, bars, hbars, stacked, area, donut, pie, scatter, gauge, heatmap, funnel", i + 1),
                    };
                    (!ok).then_some(what)
                }
                "board" => (!need("columns")).then_some("columns"),
                "table" => (!need("columns") || !need("rows")).then_some("columns and rows"),
                "flow" => (!need("steps")).then_some("steps"),
                "callout" | "text" => (!need("text") && !need("title")).then_some("text"),
                "image" => (!need("src")).then_some("src"),
                _ => None,
            };
            if let Some(m) = missing {
                warn.push(format!("row {}: {t} has no {m}", i + 1));
            }
        }
    }
    Ok(warn)
}

fn theme_named(dir: &str, name: &str) -> anyhow::Result<Value> {
    read_json(&themes_dir(dir).join(format!("{}.json", slug(name)?))).map_err(|_| anyhow!("no saved theme \"{name}\" in docs/reports/themes/"))
}

/// The report as the page draws it: theme merged (default ← saved theme ← report.theme) and tasks blocks filled.
pub fn get(dir: &str, name: &str) -> anyhow::Result<Value> {
    tasks::check_dir(dir)?;
    let name = slug(name)?;
    let path = reports_dir(dir).join(format!("{name}.json"));
    let mut report = read_json(&path)?;
    let mut theme = default_theme();
    let mut layer = |v: &Value| {
        if let (Some(t), Some(src)) = (theme.as_object_mut(), v.as_object()) {
            for (k, x) in src {
                t.insert(k.clone(), x.clone());
            }
        }
    };
    match report.get("theme") {
        Some(Value::String(n)) => layer(&theme_named(dir, n)?),
        Some(Value::Object(o)) => {
            if let Some(n) = o.get("use").and_then(Value::as_str) {
                layer(&theme_named(dir, n)?);
            }
            layer(&Value::Object(o.clone()));
        }
        _ => {}
    }
    // tasks blocks: the board's items, live
    let board = tasks::load(dir).unwrap_or(json!({"tasks": []}));
    if let Some(rows) = report.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows.iter_mut() {
            let list: Vec<&mut Value> = match row {
                Value::Array(a) => a.iter_mut().collect(),
                b => vec![b],
            };
            for b in list {
                if b.get("type").and_then(Value::as_str) == Some("tasks") {
                    let filter = b.get("filter").cloned().unwrap_or(json!({}));
                    let filter = if let Value::String(f) = filter { json!({"from": f}) } else { filter };
                    let items: Vec<Value> = tasks::select(&board, &filter).unwrap_or_default().into_iter().cloned().collect();
                    b["items"] = Value::Array(items);
                }
            }
        }
    }
    Ok(json!({"name": name, "report": report, "theme": theme, "updated": modified(&path), "file": path.display().to_string().replace('\\', "/")}))
}

pub fn write(dir: &str, name: Option<&str>, report: &Value, replace: bool) -> anyhow::Result<String> {
    tasks::check_dir(dir)?;
    let warn = validate(report)?;
    let name = slug(name.unwrap_or_else(|| report.get("title").and_then(Value::as_str).unwrap_or("")))?;
    let path = reports_dir(dir).join(format!("{name}.json"));
    if path.exists() && !replace {
        bail!("docs/reports/{name}.json exists — mode: \"replace\" to overwrite it, or another name");
    }
    let mut r = report.clone();
    r.as_object_mut().unwrap().insert("updated".into(), json!(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()));
    write_json(&path, &r)?;
    let n = blocks(&r).len();
    let mut out = format!("saved docs/reports/{name}.json · {n} block{} · the person sees it in wridge → Reports", if n == 1 { "" } else { "s" });
    if !warn.is_empty() {
        out += &format!("\nwarnings:\n{}", warn.join("\n"));
    }
    Ok(out)
}

pub fn remove(dir: &str, name: &str) -> anyhow::Result<String> {
    tasks::check_dir(dir)?;
    let name = slug(name)?;
    let path = reports_dir(dir).join(format!("{name}.json"));
    if !path.exists() {
        bail!("no report {name}");
    }
    // into docs/reports/.trash/, not gone
    tasks::keep_out_of_git(&reports_dir(dir))?;
    let trash = reports_dir(dir).join(".trash");
    std::fs::create_dir_all(&trash)?;
    let to = trash.join(format!("{name}-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    std::fs::rename(&path, &to)?;
    Ok(format!("moved to docs/reports/.trash/{}", to.file_name().unwrap().to_string_lossy()))
}

/// Reports in docs/reports/.trash/, newest first: what the Reports page can bring back.
pub fn trash(dir: &str) -> anyhow::Result<Value> {
    tasks::check_dir(dir)?;
    let mut out = vec![];
    for e in std::fs::read_dir(reports_dir(dir).join(".trash")).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let file = p.file_name().unwrap().to_string_lossy().to_string();
        // <name>-YYYYMMDD-HHMMSS.json
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let name = stem.rsplitn(3, '-').nth(2).unwrap_or(&stem).to_string();
        let title = read_json(&p).ok().and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| name.clone());
        out.push(json!({"file": file, "name": name, "title": title, "removed": modified(&p)}));
    }
    out.sort_by(|a, b| b["removed"].as_u64().cmp(&a["removed"].as_u64()));
    Ok(json!({"trash": out}))
}

/// Brings a report back from .trash under its old name (or "<name>-restored" when that name is taken again).
pub fn restore(dir: &str, file: &str) -> anyhow::Result<String> {
    tasks::check_dir(dir)?;
    if file.is_empty() || file.contains(['/', '\\']) || file.contains("..") || !file.ends_with(".json") {
        bail!("restore needs the file name from the trash list");
    }
    let from = reports_dir(dir).join(".trash").join(file);
    if !from.is_file() {
        bail!("{file} isn't in the trash");
    }
    let stem = file.trim_end_matches(".json");
    let name = stem.rsplitn(3, '-').nth(2).unwrap_or(stem).to_string();
    let mut to_name = name.clone();
    let mut n = 1;
    while reports_dir(dir).join(format!("{to_name}.json")).exists() {
        to_name = if n == 1 { format!("{name}-restored") } else { format!("{name}-restored-{n}") };
        n += 1;
    }
    std::fs::rename(&from, reports_dir(dir).join(format!("{to_name}.json")))?;
    Ok(to_name)
}

pub fn save_theme(dir: &str, name: &str, theme: &Value) -> anyhow::Result<String> {
    tasks::check_dir(dir)?;
    let o = theme.as_object().ok_or_else(|| anyhow!("theme is an object of tokens, e.g. {{\"accent\": \"#7A3BC4\"}}"))?;
    let known = default_theme();
    let unknown: Vec<&String> = o.keys().filter(|k| known.get(k.as_str()).is_none()).collect();
    if !unknown.is_empty() {
        bail!("unknown tokens: {} — known: {}", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "), known.as_object().unwrap().keys().cloned().collect::<Vec<_>>().join(", "));
    }
    let name = slug(name)?;
    write_json(&themes_dir(dir).join(format!("{name}.json")), theme)?;
    Ok(format!("saved theme \"{name}\" — use it with \"theme\": \"{name}\""))
}

// ---------------------------------------------------------------- export

/// Edge and Chrome, in that order (WRIDGE_BROWSER first when set).
fn browsers() -> Vec<PathBuf> {
    let mut c: Vec<PathBuf> = vec![];
    if let Ok(p) = std::env::var("WRIDGE_BROWSER") {
        c.push(PathBuf::from(p));
    }
    let mut bases: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"].iter().filter_map(|v| std::env::var(v).ok().map(PathBuf::from)).collect();
    // the usual places too, in case the server got a thin environment
    bases.extend([PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from(r"C:\Program Files")]);
    for app in ["Microsoft/Edge/Application/msedge.exe", "Google/Chrome/Application/chrome.exe"] {
        for b in &bases {
            c.push(b.join(app));
        }
    }
    let mut seen = std::collections::HashSet::new();
    c.into_iter().filter(|p| p.is_file() && seen.insert(p.to_string_lossy().to_lowercase().replace('\\', "/"))).collect()
}

static RUN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One headless browser run with its own fresh profile (a profile another browser holds makes the new one
/// hand off and quit at once) and none of the WebView2 settings the wridge window passes down.
async fn run_browser(exe: &Path, args: &[String]) -> anyhow::Result<std::process::Output> {
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let profile = std::env::temp_dir().join("wridge-report-browser").join(format!("{}-{n}", std::process::id()));
    let mut cmd = tokio::process::Command::new(exe);
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().to_string();
        if k.starts_with("WEBVIEW2_") || k.starts_with("CHROME_") || k.starts_with("ELECTRON_") {
            cmd.env_remove(&k);
        }
    }
    cmd.args(["--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--run-all-compositor-stages-before-draw", "--disable-background-networking"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output()).await.map_err(|_| anyhow!("the browser took too long to draw the report"));
    let _ = std::fs::remove_dir_all(&profile);
    Ok(out??)
}

/// Runs the browser until `done` says it worked: each browser twice, then the next one. The error says why.
async fn headless(args: Vec<String>, done: impl Fn(&std::process::Output) -> bool) -> anyhow::Result<String> {
    let list = browsers();
    if list.is_empty() {
        bail!("export needs Microsoft Edge or Google Chrome (set WRIDGE_BROWSER to its .exe)");
    }
    let mut why = String::new();
    for exe in &list {
        for _ in 0..2 {
            let started = std::time::Instant::now();
            match run_browser(exe, &args).await {
                Ok(out) if done(&out) => return Ok(String::from_utf8_lossy(&out.stdout).to_string()),
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    let tail: String = err.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").chars().take(240).collect();
                    why = format!("{} ended after {} ms ({}){}", exe.file_name().unwrap().to_string_lossy(), started.elapsed().as_millis(), out.status, if tail.is_empty() { String::new() } else { format!(": {tail}") });
                }
                Err(e) => why = format!("{}: {e}", exe.file_name().unwrap().to_string_lossy()),
            }
        }
    }
    bail!("couldn't draw the report — {why}")
}

/// Renders the report to PNG or PDF next to it (docs/reports/exports/); "copy" also puts the PNG on the clipboard.
pub async fn export(dir: &str, name: &str, format: &str, http_port: u16) -> anyhow::Result<Value> {
    let r = get(dir, name)?;
    let name = r["name"].as_str().unwrap().to_string();
    let width = r["theme"]["width"].as_u64().unwrap_or(1440).clamp(600, 2400);
    let q: String = url::form_urlencoded::Serializer::new(String::new()).append_pair("dir", dir).append_pair("name", &name).finish();
    let url = format!("http://127.0.0.1:{http_port}/report-view?{q}");
    // pass 1: how tall the report is (the page writes it into the DOM once fonts are in)
    let dom = headless(vec!["--virtual-time-budget=8000".into(), format!("--window-size={width},900"), "--dump-dom".into(), url.clone()], |o| String::from_utf8_lossy(&o.stdout).contains("data-height=\"")).await?;
    let height: u64 = dom.split("data-height=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("the report page didn't finish drawing"))?;
    let height = height.clamp(200, 30000);
    tasks::keep_out_of_git(&reports_dir(dir))?;
    let out_dir = reports_dir(dir).join("exports");
    std::fs::create_dir_all(&out_dir)?;
    let (ext, flag) = if format == "pdf" { ("pdf", "--print-to-pdf") } else { ("png", "--screenshot") };
    let file = out_dir.join(format!("{name}.{ext}"));
    let _ = std::fs::remove_file(&file);
    let mut args = vec!["--virtual-time-budget=8000".to_string(), format!("--window-size={width},{height}"), format!("{flag}={}", file.display())];
    if format == "pdf" {
        args.push("--no-pdf-header-footer".into());
    } else {
        args.push("--force-device-scale-factor=2".into());
    }
    args.push(format!("{url}&h={height}"));
    let target = file.clone();
    headless(args, move |_| target.is_file()).await?;
    if !file.is_file() {
        bail!("the browser didn't write {}", file.display());
    }
    if format == "copy" {
        copy_image(&file).await?;
    }
    Ok(json!({"file": file.display().to_string().replace('\\', "/"), "width": width, "height": height}))
}

#[cfg(windows)]
async fn copy_image(file: &Path) -> anyhow::Result<()> {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $i=[System.Drawing.Image]::FromFile('{}'); [System.Windows.Forms.Clipboard]::SetImage($i); $i.Dispose()",
        file.display().to_string().replace('\'', "''")
    );
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-STA", "-NonInteractive", "-Command", &script]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await.map_err(|_| anyhow!("copying took too long"))??;
    if !out.status.success() {
        bail!("couldn't copy the image: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
#[cfg(not(windows))]
async fn copy_image(_file: &Path) -> anyhow::Result<()> {
    bail!("copy image works on Windows only — use PNG")
}

/// `report` tool actions → text for the AI.
pub async fn tool(dir: &str, args: &Map<String, Value>, http_port: u16) -> anyhow::Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    match s("action").as_deref().unwrap_or("") {
        "list" => {
            let l = list(dir)?;
            let rs = l["reports"].as_array().unwrap();
            let mut out = vec![format!("{} · {} report{}", l["folder"].as_str().unwrap_or(""), rs.len(), if rs.len() == 1 { "" } else { "s" })];
            for r in rs {
                out.push(format!("{} — {} ({} blocks)", r["name"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["blocks"]));
            }
            if let Some(t) = l["themes"].as_array().filter(|a| !a.is_empty()) {
                out.push(format!("themes: {}", t.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
            }
            Ok(out.join("\n"))
        }
        "get" => {
            let name = s("name").ok_or_else(|| anyhow!("get needs name"))?;
            let p = reports_dir(dir).join(format!("{}.json", slug(&name)?));
            tasks::check_dir(dir)?;
            Ok(serde_json::to_string(&read_json(&p)?)?)
        }
        "write" => {
            let report = args.get("report").ok_or_else(|| anyhow!("write needs report: {{title, rows: [...]}}"))?;
            write(dir, s("name").as_deref(), report, s("mode").as_deref() == Some("replace"))
        }
        "remove" => remove(dir, &s("name").ok_or_else(|| anyhow!("remove needs name"))?),
        "theme" => {
            let theme = args.get("theme").ok_or_else(|| anyhow!("theme needs theme: {{token: value}} and name"))?;
            save_theme(dir, &s("name").ok_or_else(|| anyhow!("theme needs name"))?, theme)
        }
        "export" => {
            let name = s("name").ok_or_else(|| anyhow!("export needs name"))?;
            let format = s("format").unwrap_or_else(|| "png".into());
            if !["png", "pdf"].contains(&format.as_str()) {
                bail!("format: png or pdf");
            }
            let r = export(dir, &name, &format, http_port).await?;
            Ok(format!("exported {}", r["file"].as_str().unwrap_or("")))
        }
        a => bail!("unknown action {a}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_list_get_with_theme_and_tasks() {
        let td = tempfile::tempdir().unwrap();
        let d = td.path().to_str().unwrap();
        let report = json!({"title": "Phase 1", "theme": {"use": "brand", "ok": "#00AA00"}, "rows": [
            {"type": "header", "title": "Phase 1"},
            [{"type": "stats", "items": [{"label": "Tests", "value": "105"}]}, {"type": "callout", "kind": "ok", "text": "done"}],
            {"type": "tasks", "filter": "AI check"}
        ]});
        save_theme(d, "brand", &json!({"accent": "#7A3BC4"})).unwrap();
        let w = write(d, Some("Phase 1 · GitHub"), &report, false).unwrap();
        assert!(w.contains("docs/reports/phase-1-github.json · 4 blocks"), "{w}");
        assert!(write(d, Some("phase-1-github"), &report, false).unwrap_err().to_string().contains("replace"));
        tasks::apply(d, &json!({"action": "add", "tasks": [{"title": "x", "from": "AI check · 7 Nov"}, {"title": "y"}]}), "AI").unwrap();
        let g = get(d, "phase-1-github").unwrap();
        assert_eq!(g["theme"]["accent"], "#7A3BC4", "saved theme");
        assert_eq!(g["theme"]["ok"], "#00AA00", "report.theme wins");
        assert_eq!(g["theme"]["bad"], "#D23B2F", "default kept");
        let items = g["report"]["rows"][2]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "x");
        let l = list(d).unwrap();
        assert_eq!(l["reports"][0]["name"], "phase-1-github");
        assert_eq!(l["themes"][0], "brand");
        assert!(remove(d, "phase-1-github").unwrap().contains(".trash"));
        let t = trash(d).unwrap();
        let file = t["trash"][0]["file"].as_str().unwrap().to_string();
        assert_eq!(t["trash"][0]["name"], "phase-1-github");
        assert_eq!(restore(d, &file).unwrap(), "phase-1-github");
        assert!(restore(d, "../x.json").is_err());
        assert_eq!(list(d).unwrap()["reports"].as_array().unwrap().len(), 1, "restored");
    }

    #[test]
    fn validation_says_what_is_wrong() {
        let e = |r: Value| validate(&r).unwrap_err().to_string();
        assert!(e(json!({"rows": []})).contains("title"));
        assert!(e(json!({"title": "x"})).contains("rows"));
        assert!(e(json!({"title": "x", "rows": [{"type": "pie"}]})).contains("unknown block type \"pie\""));
        assert!(e(json!({"title": "x", "rows": [[{"type": "text"}, {"type": "text"}, {"type": "text"}, {"type": "text"}]]})).contains("1 to 3"));
        let w = validate(&json!({"title": "x", "rows": [{"type": "table", "columns": ["a"]}]})).unwrap();
        assert_eq!(w, ["row 1: table has no columns and rows"]);
        assert_eq!(slug("../x").unwrap(), "x");
        assert_eq!(slug("รายงาน สัปดาห์ 1").unwrap(), "รายงาน-สัปดาห์-1");
        assert!(save_theme(".", "x", &json!({"acent": "#000"})).is_err());
        // chart kinds say what they need
        let w = validate(&json!({"title": "x", "rows": [{"type": "chart", "kind": "donut", "series": []}, {"type": "chart", "kind": "gauge", "value": 72}, {"type": "chart", "items": [{"label": "a", "value": 1}]}]})).unwrap();
        assert_eq!(w, ["row 1: chart has no items"]);
        assert!(e(json!({"title": "x", "rows": [{"type": "chart", "kind": "radar"}]})).contains("unknown chart kind \"radar\""));
    }
}
