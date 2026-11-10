//! `load_test`: runs k6 on a script in the project and turns the result into a report.
//! k6 is installed by the person (not shipped: tens of MB, AGPL); wridge finds it on PATH or where
//! winget/the installer puts it. A run starts in the background (tests can take minutes); `status`
//! answers progress and, when it ends, the numbers and the report written to docs/reports/.
//! Guarded: only local or staging targets, ≤ MAX_VUS virtual users, ≤ MAX_MINUTES — more needs the
//! person's yes (`confirmed`). The script reads the base URL from `__ENV.BASE_URL`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, bail};
use serde_json::{json, Map, Value};

pub const MAX_VUS: u64 = 2000;
pub const MAX_MINUTES: u64 = 30;

/// What the person must hear before a run, and what every report ends with (scale-traffic skill §6).
pub const LIMITS: [&str; 4] = [
    "One computer simulates roughly a few thousand virtual users or a few thousand simple requests a second — more needs several machines.",
    "All requests came from one IP address: rate limits, a CDN or bot protection can block or skew them.",
    "It simulates requests, not browsers — how fast pages render and run JavaScript isn't measured here.",
    "These numbers hold for the machine and data it ran against; real people, networks and phones add variation.",
];

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------- finding k6

pub fn find_k6() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "k6.exe" } else { "k6" };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let mut more = vec![];
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        more.push(PathBuf::from(pf).join("k6").join(exe));
    }
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        more.push(PathBuf::from(la).join("Microsoft").join("WinGet").join("Links").join(exe));
    }
    more.into_iter().find(|p| p.is_file())
}

fn quiet(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(crate::util::CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

pub fn check() -> Value {
    match find_k6() {
        Some(p) => {
            let mut cmd = std::process::Command::new(&p);
            cmd.arg("version");
            quiet(&mut cmd);
            let version = cmd.output().ok().map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string()).unwrap_or_default();
            json!({"found": true, "path": p.display().to_string(), "version": version})
        }
        None => json!({"found": false, "install": "winget install k6", "docs": "https://grafana.com/docs/k6/latest/set-up/install-k6/"}),
    }
}

// ---------------------------------------------------------------- where it may run

#[derive(Debug, PartialEq)]
pub enum Place {
    Local,
    Staging,
    Other,
}

pub fn place(host: &str) -> Place {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    let local_name = h == "localhost" || h.ends_with(".localhost") || h.ends_with(".test") || h.ends_with(".local") || h == "::1" || h == "0.0.0.0";
    let private_ip = h.starts_with("127.") || h.starts_with("10.") || h.starts_with("192.168.") || {
        let mut p = h.split('.');
        p.next() == Some("172") && p.next().and_then(|x| x.parse::<u8>().ok()).map(|n| (16..=31).contains(&n)).unwrap_or(false)
    };
    if local_name || private_ip {
        return Place::Local;
    }
    const STAGING: [&str; 9] = ["staging", "stage", "stg", "dev", "test", "qa", "preview", "sandbox", "uat"];
    let labels: Vec<&str> = h.split('.').collect();
    // a label of its own (staging.example.com, app-staging.example.com), not the top-level domain (.dev)
    if labels[..labels.len().saturating_sub(1)].iter().any(|l| STAGING.iter().any(|s| *l == *s || l.starts_with(&format!("{s}-")) || l.ends_with(&format!("-{s}")))) {
        return Place::Staging;
    }
    Place::Other
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
}

/// Hosts written into the script itself (anything but `${…}`-templated URLs).
fn script_hosts(text: &str) -> Vec<String> {
    let re = regress::Regex::new(r#"https?://([A-Za-z0-9.\-\[\]:]+)"#).unwrap();
    let mut out: Vec<String> = re.find_iter(text).filter_map(|m| m.group(1).map(|g| text[g].to_string())).map(|h| h.split(':').next().unwrap_or("").to_string()).filter(|h| !h.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

/// "1h2m3.5s", "45s", "500ms" → seconds
pub fn secs(d: &str) -> f64 {
    let mut total = 0.0;
    let mut num = String::new();
    let mut chars = d.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().unwrap_or(0.0);
        num.clear();
        match c {
            'h' => total += n * 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                total += n / 1000.0
            }
            'm' => total += n * 60.0,
            's' => total += n,
            _ => {}
        }
    }
    total
}

// ---------------------------------------------------------------- runs

#[derive(Clone)]
struct Run {
    id: String,
    dir: String,
    script: String,
    target: String,
    out: PathBuf,
    started: i64,
    expected_s: f64,
    max_vus: u64,
    state: String,
    exit: Option<i32>,
    last: String,
    result: Option<Value>,
    report: Option<String>,
    error: Option<String>,
}

fn runs() -> &'static Mutex<HashMap<String, Run>> {
    static R: OnceLock<Mutex<HashMap<String, Run>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}
fn stops() -> &'static Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>> {
    static S: OnceLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn script_path(dir: &str, script: &str) -> anyhow::Result<PathBuf> {
    let base = Path::new(dir).canonicalize().map_err(|_| anyhow!("no project at {dir}"))?;
    let p = base.join(script);
    let p = p.canonicalize().map_err(|_| anyhow!("no script {script} in the project — write it first (tests/load/<flow>.js, see get_skill(\"scale-traffic\"))"))?;
    if !p.starts_with(&base) {
        bail!("the script must be inside the project");
    }
    if !matches!(p.extension().and_then(|e| e.to_str()), Some("js" | "mjs" | "ts")) {
        bail!("a k6 script is a .js (or .ts) file");
    }
    Ok(p)
}

pub async fn start(dir: &str, script: &str, target: &str, confirmed: bool) -> anyhow::Result<Value> {
    crate::tasks::check_dir(dir)?;
    let k6 = find_k6().ok_or_else(|| anyhow!("k6 isn't installed. Ask the person to run `winget install k6` (or see https://grafana.com/docs/k6/latest/set-up/install-k6/), then try again."))?;
    let path = script_path(dir, script)?;
    let host = host_of(target).ok_or_else(|| anyhow!("target must be a full URL, e.g. http://localhost:3000"))?;
    // where the traffic goes: the target, plus any address written into the script
    let text = std::fs::read_to_string(&path)?;
    let mut far: Vec<String> = vec![];
    for h in std::iter::once(host.clone()).chain(script_hosts(&text)) {
        if place(&h) == Place::Other && !far.contains(&h) {
            far.push(h);
        }
    }
    if !far.is_empty() && !confirmed {
        bail!(
            "{} isn't a local or staging address. Load-testing a live site can take it down, and testing a site that isn't the person's own is an attack. Ask them: it must be theirs, and they agree to the load at this time. Only then call again with confirmed: true.",
            far.join(", ")
        );
    }
    // how big: k6 works it out from the script's options
    let mut cmd = tokio::process::Command::new(&k6);
    cmd.args(["inspect", "--execution-requirements"]).arg("-e").arg(format!("BASE_URL={target}")).arg(&path).current_dir(dir).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(crate::util::CREATE_NO_WINDOW);
    let o = cmd.output().await?;
    if !o.status.success() {
        bail!("k6 couldn't read the script: {}", String::from_utf8_lossy(&o.stderr).lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("unknown error"));
    }
    let req: Value = serde_json::from_slice(&o.stdout).map_err(|e| anyhow!("k6 inspect: {e}"))?;
    let max_vus = req["maxVUs"].as_u64().unwrap_or(0);
    let expected_s = req["totalDuration"].as_str().map(secs).unwrap_or(0.0);
    if (max_vus > MAX_VUS || expected_s > (MAX_MINUTES * 60) as f64) && !confirmed {
        bail!(
            "This script runs up to {max_vus} virtual users for {} — above wridge's limit of {MAX_VUS} users / {MAX_MINUTES} minutes. One computer rarely simulates more reliably. Make it smaller, or ask the person and call again with confirmed: true.",
            fmt_dur(expected_s)
        );
    }
    let id = format!("lt-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let out_root = Path::new(dir).join("tests").join("load").join("out");
    crate::tasks::keep_out_of_git(&out_root)?;
    let out = out_root.join(&id);
    std::fs::create_dir_all(&out)?;
    let mut cmd = tokio::process::Command::new(&k6);
    cmd.arg("run")
        .arg("--summary-export")
        .arg(out.join("summary.json"))
        .arg("--out")
        .arg(format!("json={}", out.join("points.json.gz").display()))
        .arg("-e")
        .arg(format!("BASE_URL={target}"))
        .arg(&path)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(crate::util::CREATE_NO_WINDOW);
    let mut child = cmd.spawn().map_err(|e| anyhow!("couldn't start k6: {e}"))?;
    let run = Run {
        id: id.clone(), dir: dir.to_string(), script: script.replace('\\', "/"), target: target.to_string(), out: out.clone(), started: now(),
        expected_s, max_vus, state: "running".into(), exit: None, last: String::new(), result: None, report: None, error: None,
    };
    runs().lock().unwrap().insert(id.clone(), run);
    // the last progress line k6 prints (stderr), for status
    for pipe in [child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>), child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)].into_iter().flatten() {
        let rid = id.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut pipe = pipe;
            let mut buf = vec![0u8; 4096];
            let mut line = String::new();
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                for ch in String::from_utf8_lossy(&buf[..n]).chars() {
                    if ch == '\n' || ch == '\r' {
                        let l = line.trim().to_string();
                        if l.starts_with("running") || l.contains("level=error") || l.contains("ERRO") {
                            if let Some(r) = runs().lock().unwrap().get_mut(&rid) {
                                r.last = l.chars().take(240).collect();
                            }
                        }
                        line.clear();
                    } else {
                        line.push(ch);
                    }
                }
            }
        });
    }
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    stops().lock().unwrap().insert(id.clone(), tx);
    let rid = id.clone();
    tokio::spawn(async move {
        let stopped;
        let code = tokio::select! {
            s = child.wait() => { stopped = false; s.ok().and_then(|s| s.code()) }
            _ = rx => { stopped = true; let _ = child.kill().await; None }
        };
        stops().lock().unwrap().remove(&rid);
        let snapshot = runs().lock().unwrap().get(&rid).cloned();
        let Some(r) = snapshot else { return };
        let finished = tokio::task::spawn_blocking(move || finish(r, code, stopped)).await;
        if let Ok(r) = finished {
            runs().lock().unwrap().insert(rid, r);
        }
    });
    Ok(json!({
        "run": id, "state": "running", "expected": fmt_dur(expected_s), "maxVUs": max_vus, "target": target,
        "next": "call load_test {action: \"status\", run} in a while; it writes the report when k6 ends",
        "tellThePerson": LIMITS,
    }))
}

pub fn stop(id: &str) -> anyhow::Result<Value> {
    let tx = stops().lock().unwrap().remove(id).ok_or_else(|| anyhow!("no running load test {id}"))?;
    let _ = tx.send(());
    Ok(json!({"run": id, "state": "stopping"}))
}

pub fn status(id: &str) -> anyhow::Result<Value> {
    let map = runs().lock().unwrap();
    let r = if id.is_empty() {
        map.values().max_by_key(|r| r.started).ok_or_else(|| anyhow!("no load test has run since wridge started"))?
    } else {
        map.get(id).ok_or_else(|| anyhow!("no load test {id} (runs are kept until wridge restarts)"))?
    };
    let elapsed = (now() - r.started) as f64;
    let mut v = json!({"run": r.id, "state": r.state, "script": r.script, "target": r.target, "elapsed": fmt_dur(elapsed), "expected": fmt_dur(r.expected_s), "maxVUs": r.max_vus});
    if r.state == "running" {
        v["progress"] = json!(if r.expected_s > 0.0 { ((elapsed / r.expected_s) * 100.0).min(99.0).round() } else { 0.0 });
        v["k6"] = json!(r.last);
    }
    if let Some(x) = &r.result {
        v["result"] = x.clone();
    }
    if let Some(x) = &r.report {
        v["report"] = json!(x);
        v["next"] = json!("tell the person the result in plain words with the limits; add what you found (bottleneck, fix) to the report with report {action: \"get\"} then write mode replace");
    }
    if let Some(x) = &r.error {
        v["error"] = json!(x);
    }
    Ok(v)
}

pub fn fmt_dur(s: f64) -> String {
    let s = s.round() as u64;
    if s >= 3600 { format!("{}h {:02}m", s / 3600, (s % 3600) / 60) } else if s >= 60 { format!("{}m {:02}s", s / 60, s % 60) } else { format!("{s}s") }
}

// ---------------------------------------------------------------- reading the result

fn finish(mut r: Run, code: Option<i32>, stopped: bool) -> Run {
    r.exit = code;
    r.state = if stopped { "stopped".into() } else if code.is_some() { "done".into() } else { "failed".into() };
    match analyze(&r.out) {
        Ok(a) if a["totals"]["requests"].as_u64().unwrap_or(0) > 0 => {
            let passed = code == Some(0);
            let report = build_report(&r, &a, code, stopped);
            let name = format!("load-test-{}-{}", Path::new(&r.script).file_stem().and_then(|s| s.to_str()).unwrap_or("run"), r.id.trim_start_matches("lt-"));
            match crate::reports::write(&r.dir, Some(&name), &report, true) {
                Ok(n) => r.report = Some(n),
                Err(e) => r.error = Some(format!("couldn't write the report: {e}")),
            }
            let mut res = a;
            res["thresholds"] = json!(match code { Some(0) => "passed", Some(99) => "failed", _ => "unknown" });
            res["passed"] = json!(passed);
            r.result = Some(res);
        }
        Ok(_) => r.error = Some(format!("k6 ended (exit {}) without sending any request — check the script and BASE_URL{}", code.map(|c| c.to_string()).unwrap_or("?".into()), if r.last.is_empty() { String::new() } else { format!(". k6 said: {}", r.last) })),
        Err(e) => r.error = Some(format!("couldn't read k6's results: {e}")),
    }
    r
}

#[derive(Default)]
struct Bucket {
    durations: Vec<f32>,
    failed: u64,
    vus: u64,
}
#[derive(Default)]
struct Endpoint {
    durations: Vec<f32>,
    failed: u64,
}

fn p95(v: &mut [f32]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let i = ((v.len() as f64) * 0.95).ceil() as usize;
    v[i.saturating_sub(1).min(v.len() - 1)] as f64
}

fn endpoint_key(tags: &Value) -> String {
    if let Some(n) = tags["name"].as_str() {
        // k6 names untagged requests by their URL
        if !n.starts_with("http") {
            return n.to_string();
        }
    }
    let url = tags["url"].as_str().or(tags["name"].as_str()).unwrap_or("?");
    let path = reqwest::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_else(|_| url.split('?').next().unwrap_or(url).to_string());
    format!("{} {}", tags["method"].as_str().unwrap_or(""), path).trim().to_string()
}

/// points.json.gz (k6 --out json) → a per-second timeline and per-endpoint numbers; summary.json → totals.
pub fn analyze(out: &Path) -> anyhow::Result<Value> {
    let mut seconds: std::collections::BTreeMap<i64, Bucket> = Default::default();
    let mut ends: HashMap<String, Endpoint> = HashMap::new();
    let mut first: Option<i64> = None;
    let file = std::fs::File::open(out.join("points.json.gz"))?;
    let reader = BufReader::new(flate2::read::GzDecoder::new(file));
    for line in reader.lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        // only the metrics used, without parsing every line
        let m = if line.contains("\"metric\":\"http_req_duration\"") { 1 } else if line.contains("\"metric\":\"http_req_failed\"") { 2 } else if line.contains("\"metric\":\"vus\"") { 3 } else { 0 };
        if m == 0 || !line.contains("\"type\":\"Point\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let d = &v["data"];
        let Some(t) = d["time"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else { continue };
        let sec = t.timestamp();
        let first = *first.get_or_insert(sec);
        let b = seconds.entry(sec - first).or_default();
        let value = d["value"].as_f64().unwrap_or(0.0);
        match m {
            1 => {
                b.durations.push(value as f32);
                ends.entry(endpoint_key(&d["tags"])).or_default().durations.push(value as f32);
            }
            2 if value > 0.0 => {
                b.failed += 1;
                ends.entry(endpoint_key(&d["tags"])).or_default().failed += 1;
            }
            3 => b.vus = b.vus.max(value as u64),
            _ => {}
        }
    }
    let span = seconds.keys().last().copied().unwrap_or(0) + 1;
    // at most ~60 points on the chart
    let step = ((span as f64) / 60.0).ceil().max(1.0) as i64;
    let mut timeline = vec![];
    let mut t = 0;
    while t < span {
        let mut durs: Vec<f32> = vec![];
        let (mut failed, mut vus) = (0, 0);
        for (_, b) in seconds.range(t..t + step) {
            durs.extend_from_slice(&b.durations);
            failed += b.failed;
            vus = vus.max(b.vus);
        }
        let n = durs.len() as f64;
        timeline.push(json!({"t": t + step, "rps": (n / step as f64 * 10.0).round() / 10.0, "p95": p95(&mut durs).round(), "errorRate": if n > 0.0 { failed as f64 / n } else { 0.0 }, "vus": vus}));
        t += step;
    }
    // where it starts to slow or fail: p95 over twice the calm start (and +300 ms), or errors over 1 %
    let calm: Vec<f64> = timeline.iter().take(3).filter_map(|p| p["p95"].as_f64()).filter(|x| *x > 0.0).collect();
    let base = if calm.is_empty() { 0.0 } else { calm.iter().sum::<f64>() / calm.len() as f64 };
    let break_at = timeline.iter().position(|p| {
        let q = p["p95"].as_f64().unwrap_or(0.0);
        (base > 0.0 && q > (base * 2.0).max(base + 300.0)) || p["errorRate"].as_f64().unwrap_or(0.0) > 0.01
    });
    let mut endpoints: Vec<Value> = ends
        .into_iter()
        .map(|(k, mut e)| {
            let n = e.durations.len() as f64;
            json!({"name": k, "requests": e.durations.len(), "rps": ((n / span.max(1) as f64) * 10.0).round() / 10.0, "p95": p95(&mut e.durations).round(), "errorRate": if n > 0.0 { e.failed as f64 / n } else { 0.0 }})
        })
        .collect();
    endpoints.sort_by(|a, b| b["requests"].as_u64().cmp(&a["requests"].as_u64()));
    endpoints.truncate(15);
    // totals from k6's own summary (exact percentiles over everything)
    let s: Value = std::fs::read_to_string(out.join("summary.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let m = &s["metrics"];
    let total_reqs = m["http_reqs"]["count"].as_u64().unwrap_or_else(|| seconds.values().map(|b| b.durations.len() as u64).sum());
    let peak_rps = timeline.iter().filter_map(|p| p["rps"].as_f64()).fold(0.0, f64::max);
    let checks = (m["checks"]["passes"].as_u64().unwrap_or(0), m["checks"]["fails"].as_u64().unwrap_or(0));
    Ok(json!({
        "totals": {
            "requests": total_reqs,
            "durationS": span,
            "avgRps": (m["http_reqs"]["rate"].as_f64().unwrap_or(total_reqs as f64 / span.max(1) as f64) * 10.0).round() / 10.0,
            "peakRps": peak_rps,
            "p95": m["http_req_duration"]["p(95)"].as_f64().map(f64::round),
            "median": m["http_req_duration"]["med"].as_f64().map(f64::round),
            "max": m["http_req_duration"]["max"].as_f64().map(f64::round),
            "errorRate": m["http_req_failed"]["value"].as_f64().or(m["http_req_failed"]["rate"].as_f64()).unwrap_or(0.0),
            "maxVUs": m["vus_max"]["value"].as_u64().or(m["vus_max"]["max"].as_u64()).unwrap_or_else(|| timeline.iter().filter_map(|p| p["vus"].as_u64()).max().unwrap_or(0)),
            "checksPassed": checks.0, "checksFailed": checks.1,
        },
        "timeline": timeline,
        "breakAt": break_at,
        "endpoints": endpoints,
    }))
}

fn pct(x: f64) -> String {
    if x == 0.0 { "0%".into() } else if x < 0.001 { "<0.1%".into() } else { format!("{:.1}%", x * 100.0) }
}

fn build_report(r: &Run, a: &Value, code: Option<i32>, stopped: bool) -> Value {
    let t = &a["totals"];
    let flow = Path::new(&r.script).file_stem().and_then(|s| s.to_str()).unwrap_or("load test").to_string();
    let tl = a["timeline"].as_array().cloned().unwrap_or_default();
    let label = |p: &Value| fmt_dur(p["t"].as_f64().unwrap_or(0.0));
    let x: Vec<Value> = tl.iter().map(|p| json!(label(p))).collect();
    let col = |k: &str| -> Vec<Value> { tl.iter().map(|p| p[k].clone()).collect() };
    let marks: Vec<Value> = a["breakAt"].as_u64().map(|i| vec![json!({"x": i, "text": "slows / fails here", "color": "#D23B2F"})]).unwrap_or_default();
    let (status_text, status_kind) = if stopped { ("Stopped early", "warn") } else if code == Some(0) { ("Thresholds passed", "ok") } else if code == Some(99) { ("Thresholds failed", "bad") } else { ("Finished", "warn") };
    let err = t["errorRate"].as_f64().unwrap_or(0.0);
    let mut rows = vec![
        json!({"type": "header", "eyebrow": format!("LOAD TEST · {} · {}", r.script, chrono::Local::now().format("%-d %b %Y %H:%M")), "title": format!("{flow}: {} req/s at peak, p95 {} ms", t["peakRps"], t["p95"].as_f64().unwrap_or(0.0).round() as i64), "subtitle": format!("k6 against {} — up to {} virtual users for {}", r.target, t["maxVUs"], fmt_dur(t["durationS"].as_f64().unwrap_or(0.0))), "status": {"text": status_text, "kind": status_kind}, "meta": {"target": r.target, "script": r.script, "run": r.id}}),
        json!({"type": "stats", "items": [
            {"label": "Peak throughput", "value": format!("{} req/s", t["peakRps"]), "note": format!("avg {} req/s", t["avgRps"])},
            {"label": "p95 response", "value": format!("{} ms", t["p95"].as_f64().unwrap_or(0.0).round() as i64), "note": format!("median {} ms · max {} ms", t["median"].as_f64().unwrap_or(0.0).round() as i64, t["max"].as_f64().unwrap_or(0.0).round() as i64)},
            {"label": "Errors", "value": pct(err), "kind": if err > 0.01 { "bad" } else { "ok" }, "note": format!("{} requests", t["requests"])},
            {"label": "Virtual users", "value": format!("{}", t["maxVUs"]), "note": match (t["checksPassed"].as_u64().unwrap_or(0), t["checksFailed"].as_u64().unwrap_or(0)) { (0, 0) => "the script has no checks".to_string(), (_, 0) => format!("all {} checks passed", t["checksPassed"]), (_, f) => format!("{f} checks failed") }},
        ]}),
    ];
    if !tl.is_empty() {
        rows.push(json!([
            {"type": "chart", "kind": "line", "title": "Response time (p95)", "x": x, "series": [{"name": "p95", "values": col("p95")}], "unit": "ms", "marks": marks, "labels": "max", "span": 1},
            {"type": "chart", "kind": "line", "title": "Load", "x": x, "series": [{"name": "virtual users", "values": col("vus")}, {"name": "req/s", "values": col("rps")}], "labels": "max", "marks": marks, "span": 1},
        ]));
    }
    let ends = a["endpoints"].as_array().cloned().unwrap_or_default();
    if !ends.is_empty() {
        rows.push(json!({"type": "table", "title": "By request", "preset": "numbers",
            "columns": ["Request", {"name": "Requests", "kind": "number"}, {"name": "req/s", "kind": "number"}, {"name": "p95 (ms)", "kind": "number"}, {"name": "Errors", "kind": "number"}],
            "rows": ends.iter().map(|e| {
                let er = e["errorRate"].as_f64().unwrap_or(0.0);
                let cells = json!([e["name"], e["requests"], e["rps"], e["p95"], pct(er)]);
                if er > 0.01 { json!({"cells": cells, "bg": "#FBE7E4", "ink": "#A42C22"}) } else { cells }
            }).collect::<Vec<_>>()}));
    }
    if let Some(i) = a["breakAt"].as_u64() {
        let p = &tl[i as usize];
        rows.push(json!({"type": "callout", "kind": "warn", "title": format!("It started to slow down at {} virtual users, {} req/s", p["vus"], p["rps"]), "text": format!("At {} into the test p95 reached {} ms with {} errors. Look at the slowest request in the table first; scale-traffic §2 lists what to try, in order.", label(p), p["p95"].as_f64().unwrap_or(0.0).round() as i64, pct(p["errorRate"].as_f64().unwrap_or(0.0)))}));
    }
    rows.push(json!({"type": "callout", "kind": "info", "title": "What this test can't tell you", "text": LIMITS.iter().map(|l| format!("• {l}")).collect::<Vec<_>>().join("\n")}));
    json!({"title": format!("Load test · {flow}"), "rows": rows})
}

/// The tool: check · run · status · stop.
pub async fn tool(args: &Map<String, Value>) -> anyhow::Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let v = match s("action").as_str() {
        "check" => check(),
        "status" => status(&s("run"))?,
        "stop" => stop(&s("run"))?,
        _ => {
            if s("script").is_empty() || s("target").is_empty() {
                bail!("run needs script (e.g. tests/load/booking.js) and target (e.g. http://localhost:3000)");
            }
            start(&s("projectDir"), &s("script"), &s("target"), args.get("confirmed").and_then(Value::as_bool).unwrap_or(false)).await?
        }
    };
    Ok(serde_json::to_string_pretty(&v)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn places() {
        assert_eq!(place("localhost"), Place::Local);
        assert_eq!(place("127.0.0.1"), Place::Local);
        assert_eq!(place("[::1]"), Place::Local);
        assert_eq!(place("192.168.1.20"), Place::Local);
        assert_eq!(place("172.20.0.3"), Place::Local);
        assert_eq!(place("app.test"), Place::Local);
        assert_eq!(place("staging.example.com"), Place::Staging);
        assert_eq!(place("api-staging.example.com"), Place::Staging);
        assert_eq!(place("example.dev"), Place::Other, ".dev as a top-level domain is not staging");
        assert_eq!(place("172.40.0.1"), Place::Other);
        assert_eq!(place("www.google.com"), Place::Other);
    }

    #[test]
    fn durations_and_hosts() {
        assert_eq!(secs("1m30s"), 90.0);
        assert_eq!(secs("2h0m0s"), 7200.0);
        assert_eq!(secs("500ms"), 0.5);
        assert_eq!(script_hosts("http.get(`${BASE}/x`); http.get('https://api.example.com/a'); const u = \"http://localhost:3000\""), vec!["api.example.com", "localhost"]);
    }

    #[test]
    fn reads_k6_points_into_a_timeline() {
        let dir = std::env::temp_dir().join(format!("wr-k6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut gz = flate2::write::GzEncoder::new(std::fs::File::create(dir.join("points.json.gz")).unwrap(), flate2::Compression::fast());
        // 10 seconds: calm for 5 (100 ms), then slow (900 ms) with failures on /book
        for s in 0..10 {
            let t = format!("2026-11-10T10:00:{:02}.500+07:00", s);
            writeln!(gz, r#"{{"type":"Point","metric":"vus","data":{{"time":"{t}","value":{},"tags":{{}}}}}}"#, (s + 1) * 10).unwrap();
            for i in 0..20 {
                let (name, ms, fail) = if i % 2 == 0 { ("home", 80.0, 0) } else { ("book", if s < 5 { 100.0 } else { 900.0 }, if s >= 5 && i % 3 == 0 { 1 } else { 0 }) };
                writeln!(gz, r#"{{"type":"Point","metric":"http_req_duration","data":{{"time":"{t}","value":{ms},"tags":{{"name":"{name}","method":"GET"}}}}}}"#).unwrap();
                writeln!(gz, r#"{{"type":"Point","metric":"http_req_failed","data":{{"time":"{t}","value":{fail},"tags":{{"name":"{name}","method":"GET"}}}}}}"#).unwrap();
            }
        }
        gz.finish().unwrap();
        std::fs::write(dir.join("summary.json"), r#"{"metrics":{"http_reqs":{"count":200,"rate":20},"http_req_duration":{"p(95)":900,"med":90,"max":900},"http_req_failed":{"value":0.0333},"vus_max":{"value":100}}}"#).unwrap();
        let a = analyze(&dir).unwrap();
        assert_eq!(a["totals"]["requests"], 200);
        assert_eq!(a["timeline"].as_array().unwrap().len(), 10);
        assert_eq!(a["breakAt"], 5, "slows at second 6");
        let book = a["endpoints"].as_array().unwrap().iter().find(|e| e["name"] == "book").unwrap();
        assert_eq!(book["p95"], 900.0);
        assert!(book["errorRate"].as_f64().unwrap() > 0.0);
        // the report it makes passes the report checks
        let r = Run { id: "lt-x".into(), dir: dir.display().to_string(), script: "tests/load/booking.js".into(), target: "http://localhost:3000".into(), out: dir.clone(), started: 0, expected_s: 10.0, max_vus: 100, state: "done".into(), exit: Some(99), last: String::new(), result: None, report: None, error: None };
        let rep = build_report(&r, &a, Some(99), false);
        crate::reports::validate(&rep).unwrap();
        // WR_DUMP_REPORT=<project dir>: also write it there, to look at in wridge → Reports
        if let Ok(to) = std::env::var("WR_DUMP_REPORT") {
            crate::reports::write(&to, Some("load-test-sample"), &rep, true).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
