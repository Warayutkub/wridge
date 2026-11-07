//! The HTTP side of index.mjs: host/origin guards and CORS, the pages and static files, every /api route,
//! /guide and /mcp. The code graph, brain and flows routes call wridge-graph (through `codegraph`), the
//! design-side ones wridge-figma.

use std::time::Duration;

use anyhow::anyhow;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use serde_json::{json, Map, Value};

use wridge_figma::{js as fjs, lottie, motion, motion_figma, motion_handoff, node_path, obj, pdf};
use wridge_graph as wg;

use crate::app::{tool_schema, Ctx, VERSION};
use crate::jsjson::{buffer_from_base64, first_of, js_string, js_trim, prop, stringify, truthy};
use crate::codegraph::{self, blocking};
use crate::util::{log, read_file, slash};
use crate::zod;

/// Browser bundles served to graph.html (allowlist — never map arbitrary paths). Copied from node_modules
/// into server/app/vendor/.
pub const VENDOR_FILES: &[&str] = &[
    "3d-force-graph.min.js",
    "force-graph.min.js",
    "three.module.js",
    "three.core.js",
    "lottie.min.js",
    "gifenc.esm.js",
];

/// Web pages and their shared UI kit, read per request so edits apply without a restart.
pub const PAGES: &[(&str, &str, &str)] = &[
    ("/", "dashboard.html", "text/html"),
    ("/graph", "graph.html", "text/html"),
    ("/ui.css", "ui.css", "text/css"),
    ("/ui.js", "ui.js", "text/javascript"),
    ("/code", "code.html", "text/html"),
    ("/data", "data.html", "text/html"),
    ("/flows", "flows.html", "text/html"),
    ("/github", "github.html", "text/html"),
    ("/tasks", "tasks.html", "text/html"),
    ("/reports", "reports.html", "text/html"),
    ("/report-view", "report-view.html", "text/html"),
    ("/report-render.js", "report-render.js", "text/javascript"),
    ("/skills", "skills.html", "text/html"),
    ("/motion", "motion.html", "text/html"),
    ("/pdf", "pdf.html", "text/html"),
    ("/brain", "brain.html", "text/html"),
    // the wridge window itself, also openable in a browser for a look (no Tauri there: start/stop are off)
    ("/app", "app.html", "text/html"),
    ("/app.css", "app.css", "text/css"),
    ("/app.js", "app.js", "text/javascript"),
    ("/app-icons.js", "app-icons.js", "text/javascript"),
    ("/mindmap.js", "mindmap.js", "text/javascript"),
    ("/graph3d.js", "graph3d.js", "text/javascript"),
    ("/search.js", "search.js", "text/javascript"),
];

pub fn json_response(status: u16, body: &Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(stringify(body)))
        .unwrap()
}

/// json() for values built from wridge-figma results: exactly the bytes `JSON.stringify` writes.
fn js_json_response(status: u16, body: &Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(fjs::stringify(body)))
        .unwrap()
}

/// json() for values built from wridge-graph results (`wridge_graph::js::json_stringify`).
fn graph_json(status: u16, body: &Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(codegraph::stringify(body)))
        .unwrap()
}

fn error_json(status: u16, message: &str) -> Response {
    json_response(status, &json!({"error": message}))
}

fn bytes_response(content_type: &str, cache: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(body))
        .unwrap()
}

pub struct Req {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub body: Body,
}

impl Req {
    /// `searchParams.get(name)`
    pub fn param(&self, name: &str) -> Option<String> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    }

    /// readJson(req, limit): the body as JSON, `{}` when empty.
    async fn json(&mut self, limit: usize) -> anyhow::Result<Value> {
        let body = std::mem::replace(&mut self.body, Body::empty());
        let bytes = axum::body::to_bytes(body, limit).await.map_err(|_| anyhow!("Request body too large"))?;
        if bytes.is_empty() {
            return Ok(json!({}));
        }
        crate::jsjson::parse(&String::from_utf8_lossy(&bytes)).map_err(|m| anyhow!(m))
    }
}

/// `const { a, b } = value` — throws like JS when the value is null.
fn destructure(v: &Value, first: &str) -> anyhow::Result<Map<String, Value>> {
    match v {
        Value::Null => Err(anyhow!("Cannot destructure property '{first}' of '(intermediate value)' as it is null.")),
        Value::Object(o) => Ok(o.clone()),
        _ => Ok(Map::new()),
    }
}

const BODY_LIMIT: usize = 1_000_000;

pub async fn entry(State(app): State<Ctx>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let host = parts.headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let allowed_hosts = [format!("localhost:{}", app.http_port), format!("127.0.0.1:{}", app.http_port)];
    // Host check blocks DNS rebinding; Origin check blocks other websites (Claude/CLI send no Origin).
    if !allowed_hosts.contains(&host) {
        return error_json(403, "forbidden host");
    }
    let origin = parts.headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    if let Some(o) = &origin {
        // + the wridge app's control window (Tauri serves it from tauri.localhost on Windows)
        let allowed = allowed_hosts.iter().any(|h| *o == format!("http://{h}")) || o == "http://tauri.localhost" || o == "tauri://localhost";
        if !allowed {
            return error_json(403, "forbidden origin");
        }
    }
    let mut response = if parts.method == Method::OPTIONS {
        Response::builder()
            .status(204)
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header("Access-Control-Allow-Headers", "Content-Type, X-Wridge-Session")
            .header("Access-Control-Max-Age", "600")
            .body(Body::empty())
            .unwrap()
    } else {
        let target = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        let Ok(url) = url::Url::parse(&format!("http://{host}{target}")) else {
            return error_json(400, "bad request");
        };
        let req = Req {
            method: parts.method.clone(),
            path: url.path().to_string(),
            query: url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
            headers: parts.headers.clone(),
            body,
        };
        let route = format!("{} {}", req.method, req.path);
        // every route runs as the caller's session (session.rs): the wridge window and its pages send "app"
        let session = crate::session::parse(parts.headers.get(crate::session::HEADER).and_then(|v| v.to_str().ok())).unwrap_or_else(|| std::sync::Arc::from(crate::session::SHARED));
        match crate::session::scope(session, dispatch(&app, req)).await {
            Ok(r) => r,
            Err(err) => {
                log(&format!("{route} failed: {err}"));
                error_json(500, &err.to_string())
            }
        }
    };
    // the wridge window (another origin) reads the API: answer allowed origins with CORS
    if let Some(o) = origin {
        if let Ok(v) = HeaderValue::from_str(&o) {
            response.headers_mut().insert("Access-Control-Allow-Origin", v);
            response.headers_mut().insert(header::VARY, HeaderValue::from_static("Origin"));
        }
    }
    response
}

/// `if (!dir || !isKnownProject(dir)) return json(res, 404, { error })` for `?dir=`: the folder, or the answer.
async fn known_query_dir(app: &Ctx, dir: Option<String>, message: &str) -> anyhow::Result<Result<String, Response>> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else {
        return Ok(Err(error_json(404, message)));
    };
    if !codegraph::known(&app.paths.projects_file, &Value::String(dir.clone())).await? {
        return Ok(Err(error_json(404, message)));
    }
    Ok(Ok(dir))
}

/// The same check for `dir` from a JSON body (`isKnownProject` throws for a value that is not a string).
/// Past it, `dir` is a string.
async fn known_body_dir(app: &Ctx, dir: Option<&Value>, message: &str) -> anyhow::Result<Option<Response>> {
    match dir {
        Some(d) if truthy(dir) && codegraph::known(&app.paths.projects_file, d).await? => Ok(None),
        _ => Ok(Some(error_json(404, message))),
    }
}

const ADD_FIRST: &str = "Add this project in the code graph page first";
const ADD_BRAIN: &str = "Add this project first";

/// `/api/tasks` and `/api/reports*`: the Tasks and Reports pages (the same code as the `tasks` / `report` tools;
/// the window acts as "you").
async fn work_route(app: &Ctx, mut req: Req) -> anyhow::Result<Response> {
    let dir = req.param("dir").unwrap_or_default();
    let post = req.method == Method::POST;
    let fail = |e: anyhow::Error| json_response(400, &json!({"error": e.to_string()}));
    match (post, req.path.as_str()) {
        (false, "/api/tasks") => {
            let d = dir.clone();
            let r = crate::codegraph::blocking(move || {
                crate::tasks::check_dir(&d)?;
                Ok(json!({"board": crate::tasks::load(&d)?, "stamp": crate::tasks::stamp(&d), "file": crate::tasks::board_path(&d).display().to_string().replace('\\', "/")}))
            })
            .await;
            Ok(r.map(|v| json_response(200, &v)).unwrap_or_else(fail))
        }
        (true, "/api/tasks") => {
            let body = req.json(BODY_LIMIT).await?;
            let d = dir.clone();
            let r = crate::codegraph::blocking(move || {
                let (msg, board) = crate::tasks::apply(&d, &body, "you")?;
                Ok(json!({"message": msg, "board": board, "stamp": crate::tasks::stamp(&d)}))
            })
            .await;
            Ok(r.map(|v| json_response(200, &v)).unwrap_or_else(fail))
        }
        // the pages wait here until the AI (or another window) changes something — up to 25 s, then they ask again.
        // Answers come from the network, not from timers a hidden page slows down.
        (false, "/api/tasks/wait") | (false, "/api/reports/wait") => {
            let since = req.param("since").unwrap_or_default();
            let tasks = req.path == "/api/tasks/wait";
            let sig = |d: &str| if tasks { crate::tasks::stamp(d).to_string() } else { crate::reports::signature(d) };
            let start = std::time::Instant::now();
            loop {
                let now = sig(&dir);
                if now != since || start.elapsed() > std::time::Duration::from_secs(25) {
                    return Ok(json_response(200, &json!({"since": now, "changed": now != since})));
                }
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
        }
        (false, "/api/reports") => Ok(crate::reports::list(&dir).map(|v| json_response(200, &v)).unwrap_or_else(fail)),
        (false, "/api/reports/get") => {
            let name = req.param("name").unwrap_or_default();
            Ok(crate::reports::get(&dir, &name).map(|v| json_response(200, &v)).unwrap_or_else(fail))
        }
        (true, "/api/reports/export") => {
            let b = req.json(BODY_LIMIT).await?;
            let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let format = b.get("format").and_then(Value::as_str).unwrap_or("png").to_string();
            if !["png", "pdf", "copy"].contains(&format.as_str()) {
                return Ok(error_json(400, "format: png, pdf or copy"));
            }
            Ok(crate::reports::export(&dir, &name, &format, app.http_port).await.map(|v| json_response(200, &v)).unwrap_or_else(fail))
        }
        // an image a report shows (inside the project only)
        (false, "/api/reports/file") => {
            let rel = req.param("path").unwrap_or_default();
            let base = std::path::Path::new(&dir);
            let ok_ext = ["png", "jpg", "jpeg", "gif", "webp", "svg"];
            let file = base.join(&rel);
            let (Ok(b), Ok(f)) = (base.canonicalize(), file.canonicalize()) else { return Ok(error_json(404, "no such file")) };
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if !f.starts_with(&b) || !ok_ext.contains(&ext.as_str()) {
                return Ok(error_json(403, "only images inside the project"));
            }
            let kind = match ext.as_str() { "svg" => "image/svg+xml", "jpg" | "jpeg" => "image/jpeg", e => if e == "png" { "image/png" } else if e == "gif" { "image/gif" } else { "image/webp" } };
            Ok(bytes_response(kind, "no-cache", std::fs::read(&f)?))
        }
        (true, "/api/reports/remove") => {
            let b = req.json(BODY_LIMIT).await?;
            let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            Ok(crate::reports::remove(&dir, &name).map(|m| json_response(200, &json!({"message": m}))).unwrap_or_else(fail))
        }
        _ => Ok(error_json(404, "not found")),
    }
}

/// `/api/github/*` (docs/flows/github.md): the window's only way to GitHub; tokens never leave the server.
async fn github_route(app: &Ctx, mut req: Req) -> anyhow::Result<Response> {
    let gh = &app.github;
    let path = req.path.trim_start_matches("/api/github").to_string();
    let ok = |v: anyhow::Result<Value>| match v {
        Ok(v) => json_response(200, &v),
        Err(e) => json_response(502, &json!({"error": e.to_string(), "status": gh.status()})),
    };
    let post = req.method == Method::POST;
    if req.method != Method::GET && !post {
        return Ok(error_json(405, "method not allowed"));
    }
    match (post, path.as_str()) {
        (false, "/status") => return Ok(json_response(200, &gh.status())),
        (true, "/signin") => return Ok(ok(gh.sign_in().await)),
        (true, "/signin/cancel") => {
            gh.cancel_sign_in();
            return Ok(json_response(200, &gh.status()));
        }
        (true, "/signout") => {
            gh.sign_out();
            return Ok(json_response(200, &gh.status()));
        }
        (true, "/notify") => {
            let b = req.json(BODY_LIMIT).await?;
            return Ok(ok(gh.set_notify(b.get("failed").and_then(Value::as_bool), b.get("succeeded").and_then(Value::as_bool))));
        }
        (true, "/open") => {
            let b = req.json(BODY_LIMIT).await?;
            let url = b.get("url").and_then(Value::as_str).unwrap_or("");
            return Ok(match gh.open(url) {
                Ok(()) => json_response(200, &json!({"opened": url})),
                Err(e) => error_json(400, &e.to_string()),
            });
        }
        (false, "/repos") => return Ok(ok(gh.repos().await)),
        _ => {}
    }
    // /repo/{owner}/{name}/runs · /runs/{id} · /jobs/{id}/log
    if let (false, Some(rest)) = (post, path.strip_prefix("/repo/")) {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 3 {
            let repo = format!("{}/{}", parts[0], parts[1]);
            if !crate::github::valid_repo(&repo) {
                return Ok(error_json(400, "not a repo"));
            }
            let id = |s: &str| s.parse::<u64>().ok();
            match &parts[2..] {
                ["runs"] => return Ok(ok(gh.runs(&repo).await)),
                ["runs", n] if id(n).is_some() => return Ok(ok(gh.run(&repo, id(n).unwrap()).await)),
                ["jobs", n, "log"] if id(n).is_some() => return Ok(ok(gh.log(&repo, id(n).unwrap()).await)),
                _ => {}
            }
        }
    }
    Ok(error_json(404, "not found"))
}

async fn dispatch(app: &Ctx, mut req: Req) -> anyhow::Result<Response> {
    let path = req.path.clone();
    let get = req.method == Method::GET;

    if path == "/mcp" {
        let profile = req.param("profile").unwrap_or_else(|| "core".into());
        return Ok(crate::mcp::handle(app, req, &profile).await);
    }

    if get {
        if let Some(name) = path.strip_prefix("/vendor/") {
            if VENDOR_FILES.contains(&name) {
                let bytes = read_file(&app.paths.vendor.join(name))?;
                return Ok(bytes_response("text/javascript; charset=utf-8", "max-age=86400", bytes));
            }
        }
        // wara-bridge's own Lottie art (web/lottie/<name>.json) for the pages' loading / success / empty states.
        let file_re = |prefix: &str, ext: &str| -> Option<String> {
            let re = regress::Regex::new(&format!("^\\/{prefix}\\/([a-z0-9-]+)\\.{ext}$")).unwrap();
            re.find(&path).map(|m| path[m.group(1).unwrap()].to_string())
        };
        if let Some(name) = file_re("lottie", "json") {
            let file = app.paths.web.join("lottie").join(format!("{name}.json"));
            if !file.exists() {
                return Ok(error_json(404, "No such animation"));
            }
            return Ok(bytes_response("application/json; charset=utf-8", "no-cache", read_file(&file)?));
        }
        // The user's sticker set (web/stickers/<mood>.png) for empty / error / waiting states.
        if let Some(name) = file_re("stickers", "png") {
            let file = app.paths.web.join("stickers").join(format!("{name}.png"));
            if !file.exists() {
                return Ok(error_json(404, "No such sticker"));
            }
            return Ok(bytes_response("image/png", "max-age=86400", read_file(&file)?));
        }
        if let Some((_, file, kind)) = PAGES.iter().find(|(p, _, _)| *p == path) {
            let bytes = read_file(&app.paths.web.join(file))?;
            return Ok(bytes_response(
                &format!("{kind}; charset=utf-8"),
                "no-store",
                String::from_utf8_lossy(&bytes).into_owned().into_bytes(),
            ));
        }
    }

    if path == "/api/github" || path.starts_with("/api/github/") {
        return github_route(app, req).await;
    }
    if path == "/api/tasks" || path == "/api/tasks/wait" || path == "/api/reports" || path.starts_with("/api/reports/") {
        return work_route(app, req).await;
    }

    let route = format!("{} {}", req.method, path);
    match route.as_str() {
        // ---- code graph page (works without Figma) ----
        "GET /api/code/projects" => {
            // codeFiles: parsed source files (0 for a book or a game the graph cannot read) — the menu hides the code pages then.
            let file = app.paths.projects_file.clone();
            Ok(graph_json(200, &blocking(move || codegraph::projects_view(&file)).await?))
        }

        "POST /api/code/projects" => {
            let body = req.json(BODY_LIMIT).await?;
            let dir = destructure(&body, "dir")?.get("dir").cloned();
            let file = app.paths.projects_file.clone();
            let info = blocking(move || {
                let dir = codegraph::project_dir_arg(dir.as_ref())?;
                // validates the path; adds .wara/ to .gitignore before creating it
                let info = wg::update_graph(&dir, wg::UpdateOptions::default())?;
                wg::projects::add_project(&file, &dir)?;
                Ok(info)
            })
            .await?;
            Ok(graph_json(200, &info))
        }

        "POST /api/code/projects/remove" => {
            // Forget the project in this list only; its .wara/ folder stays.
            let body = req.json(BODY_LIMIT).await?;
            if body.is_null() {
                return Err(anyhow!("Cannot read properties of null (reading 'dir')"));
            }
            let dir = body.as_object().and_then(|o| o.get("dir")).cloned();
            let file = app.paths.projects_file.clone();
            blocking(move || codegraph::remove(&file, dir.as_ref())).await?;
            Ok(json_response(200, &json!({"ok": true})))
        }

        "GET /api/code/graph" | "GET /api/code/data" => {
            let dir = match known_query_dir(app, req.param("dir"), ADD_FIRST).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let data = path == "/api/code/data";
            let out = blocking(move || {
                let (db, info) = wg::open_graph(&dir)?;
                let view = if data { wg::export::export_data(&db, &dir)? } else { wg::export::export_graph(&db, &dir)? };
                Ok(codegraph::with(view, "update", info))
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        // ---- flows page: docs/flows/*.md + drafts + live trace ----
        "GET /api/motion" => {
            // Lottie files in the project (found by content) + the motion tokens the code already defines.
            let dir = match known_query_dir(app, req.param("dir"), ADD_FIRST).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let root = codegraph::project_root(&dir)?;
            let r = root.clone();
            // no graph yet: the page shows the defaults
            let tokens = blocking(move || motion_tokens(&dir, &r)).await.unwrap_or_default();
            let r = root.clone();
            let files = blocking(move || motion::list_lottie(&r)).await?;
            let mut out = Map::new();
            out.insert("name".into(), codegraph::base_name(&root).into());
            out.insert("files".into(), Value::Array(files));
            out.insert("tokens".into(), Value::Array(tokens));
            out.insert("figmaFile".into(), app.bridge.file_name());
            out.insert("plugin".into(), app.bridge.plugin_connected());
            Ok(graph_json(200, &Value::Object(out)))
        }

        "GET /api/motion/file" => {
            let dir = match known_query_dir(app, req.param("dir"), ADD_FIRST).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let root = codegraph::project_root(&dir)?;
            let p = req.param("path").unwrap_or_default();
            Ok(js_json_response(200, &blocking(move || motion::read_lottie(&root, &p)).await?))
        }

        "GET /api/flows" => {
            let dir = match known_query_dir(app, req.param("dir"), ADD_FIRST).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let out = blocking(move || {
                let (db, info) = wg::open_graph(&dir)?;
                let root = codegraph::project_root(&dir)?;
                let overview = wg::flows::flows_overview(&wg::query::graph(&db)?, &root)?;
                Ok(codegraph::named(codegraph::base_name(&root), overview, info))
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        "GET /api/flows/trace" => {
            let dir = match known_query_dir(app, req.param("dir"), ADD_FIRST).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let target = req.param("target").unwrap_or_default();
            let out = blocking(move || {
                let (db, _) = wg::open_graph(&dir)?;
                wg::flows::trace(&wg::query::graph(&db)?, &target, 3, 40)
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        "GET /api/motion/figma-frames" => {
            // Frames on the open Figma page that have a Figma Motion timeline, with the code-or-Lottie verdict.
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            let doc = app.raw("get_document_info", json!({}), None).await?;
            let mut frames: Vec<Value> = vec![];
            // frames on the page, and frames inside sections (where storyboards and flows live)
            let mut candidates: Vec<Value> = vec![];
            for top in doc.get("topLevel").and_then(Value::as_array).cloned().unwrap_or_default() {
                if top.get("type") != Some(&json!("SECTION")) {
                    candidates.push(top);
                    continue;
                }
                if let Ok(node) = app.raw("get_node", json!({"nodeId": top.get("id"), "depth": 1}), None).await {
                    for c in node.get("children").and_then(Value::as_array).cloned().unwrap_or_default() {
                        let mut c = c.as_object().cloned().unwrap_or_default();
                        c.insert("section".into(), top.get("name").cloned().unwrap_or(Value::Null));
                        candidates.push(Value::Object(c));
                    }
                }
            }
            for top in candidates {
                let kind = js_string(top.get("type"));
                if !["FRAME", "COMPONENT", "INSTANCE", "GROUP"].contains(&kind.as_str()) {
                    continue;
                }
                let Ok(r) = app.raw("motion_read", json!({"nodeId": top.get("id")}), None).await else {
                    continue;
                };
                let Some(animated) = r.get("animated").and_then(Value::as_array) else {
                    return Err(anyhow!("Cannot read properties of undefined (reading 'some')"));
                };
                // `n.tracks.length` truthy: a non-empty array (or string)
                let any_tracks = animated.iter().any(|n| match n.get("tracks") {
                    Some(Value::Array(t)) => !t.is_empty(),
                    Some(Value::String(s)) => !s.is_empty(),
                    _ => false,
                });
                if !any_tracks {
                    continue;
                }
                let section = top.get("section");
                let context = motion_handoff::motion_context(section);
                let rows = motion_handoff::classify(&r, &mut vec![], context)?;
                let size = |k: &str| fjs::round(fjs::to_number(top.get(k)));
                let duration = r.get("timelines").and_then(|t| t.get(0)).and_then(|t| fjs::nullish(t.get("duration")));
                let listed: Vec<Value> = rows
                    .iter()
                    .map(|x| {
                        obj! {
                            "id" => x.id.clone(), "name" => x.name.clone(), "nth" => x.nth.clone(), "to" => x.to.as_str(), "why" => x.why.as_str(),
                            "props" => Value::Array(x.props.iter().map(|p| Value::String(p.clone())).collect()), "span" => fjs::nums(&x.span),
                        }
                    })
                    .collect();
                let lint: Vec<Value> = motion_handoff::lint(&rows, &r, context).into_iter().map(Value::String).collect();
                frames.push(obj! {
                    "id" => top.get("id").cloned(), "name" => top.get("name").cloned(), "w" => size("width"), "h" => size("height"),
                    "durationMs" => fjs::round(duration.map_or(0.0, fjs::number) * 1000.0),
                    "rows" => listed, "lint" => lint, "context" => context, "section" => fjs::nullish(section).cloned().unwrap_or(Value::Null),
                });
            }
            let mut page = file_page(&doc, "frames");
            page["frames"] = Value::Array(frames);
            Ok(js_json_response(200, &page))
        }

        "GET /api/motion/figma-preview" => {
            // A Lottie built in memory from a frame's Motion timeline — to watch it here, nothing is written.
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            let Some(id) = req.param("id").filter(|s| !s.is_empty()) else {
                return Ok(error_json(400, "id is required"));
            };
            let r = app.raw("motion_read", json!({"nodeId": id}), None).await?;
            let spec = motion_figma::motion_to_spec(&r)?;
            let asset = first_of(app.raw("export_assets", json!({"items": [{"nodeId": id, "format": "svg", "scale": 1}]}), None).await?)?;
            let name = prop(prop(Some(&r), "root")?, "name")?.cloned();
            let svg = String::from_utf8_lossy(&buffer_from_base64(prop(asset.as_ref(), "base64")?)?).into_owned();
            let compiled = lottie::compile_lottie(&obj! {
                "duration" => spec.get("duration").cloned(), "tracks" => spec.get("tracks").cloned(), "split" => spec.get("split").cloned(),
                "name" => name, "svg" => svg,
            })?;
            let mut lottie = compiled.get("lottie").cloned().unwrap_or(Value::Null);
            if let Value::Object(o) = &mut lottie {
                let recipe = obj! { "duration" => spec.get("duration").cloned(), "tracks" => spec.get("tracks").cloned(), "split" => spec.get("split").cloned(), "nodeId" => id };
                o.insert(
                    "meta".into(),
                    obj! { "g" => "wara-bridge", "wara" => obj! { "source" => "figma-motion", "spec" => recipe } },
                );
            }
            let warnings: Vec<Value> = [spec.get("warnings"), compiled.get("warnings")]
                .into_iter()
                .flat_map(|w| w.and_then(Value::as_array).cloned().unwrap_or_default())
                .collect();
            Ok(js_json_response(200, &obj! { "lottie" => lottie, "warnings" => warnings }))
        }

        "POST /api/pick-folder" => {
            // Windows' own "Select Folder" dialog (File Explorer style, folder-picker.ps1): a browser can pick a
            // folder but never tells the real path.
            if !cfg!(windows) {
                return Ok(error_json(501, "Folder dialog is only available on Windows"));
            }
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "start")?;
            let or = |v: Option<&Value>, d: &str| match v {
                None | Some(Value::Null) => d.to_string(),
                Some(x) => js_string(Some(x)),
            };
            let title = or(b.get("title"), "Choose a folder");
            let start = or(b.get("start"), "");
            let script = app.paths.folder_picker.display().to_string();
            let picked = tokio::task::spawn_blocking(move || {
                let out = crate::util::hidden_command("powershell.exe")
                    .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File", &script, "-Title", &title, "-Start", &start])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output();
                match out {
                    Ok(o) => {
                        let s = String::from_utf8_lossy(&o.stdout).into_owned();
                        let t = js_trim(&s).to_string();
                        if t.is_empty() {
                            None
                        } else {
                            Some(t)
                        }
                    }
                    Err(_) => None,
                }
            })
            .await
            .unwrap_or(None);
            Ok(json_response(200, &json!({"path": picked.map(|p| p.replace('\\', "/"))})))
        }

        "POST /api/motion/figma-to-lottie" => {
            // The page's "convert" button: the same motion_to_lottie the AI uses, written into the project.
            // The page asks before replacing a file that exists (overwrite: true).
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            figma_to_lottie(app, &b).await
        }

        "POST /api/motion/figma" => {
            // A GIF of the animation (made in the page) into a Lottie/<name> component, so it moves in Figma's
            // Present view. The page first asks which file is open (no confirmFile) and confirms it with the user.
            let body = req.json(30_000_000).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected — open the design file and run the wridge plugin"));
            }
            motion_to_figma(app, &b).await
        }

        "PUT /api/flows" => {
            // Save one flow file. baseHash = the file's hash when it was opened (null for a new file): if the
            // file changed meanwhile (AI, VS Code, a teammate) nothing is overwritten — 409 with the current text.
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            save_flow(&b).await
        }

        "POST /api/flows/ignore" => {
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            let dir = b.get("dir");
            if !truthy(dir) || !codegraph::known(&app.paths.projects_file, dir.unwrap()).await? || !truthy(b.get("id")) {
                return Ok(error_json(400, "dir and id are required"));
            }
            let dir = js_string(dir);
            let id = js_string(b.get("id"));
            blocking(move || wg::flows::ignore_draft(&codegraph::project_root(&dir)?, &id)).await?;
            Ok(json_response(200, &json!({"ok": true})))
        }

        "POST /api/code/design" => {
            // Copy the open Figma file's components + tokens into this project's graph.
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected — open the design file and run the wridge plugin"));
            }
            let dir = js_string(b.get("dir"));
            let (db, _) = blocking(move || wg::open_graph(&dir)).await?;
            let info = app.raw("get_document_info", json!({}), None).await?;
            let components = app.raw("get_components", json!({}), None).await?;
            let data = app.raw("get_tokens", json!({"source": "auto"}), Some(120_000)).await?;
            let out = blocking(move || wg::design::sync_design_from(&db, &info, &components, &data)).await?;
            Ok(graph_json(200, &out))
        }

        "POST /api/code/open" => {
            // Open a project file in VS Code from the server (a vscode:// link makes the browser ask for permission every time).
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            let root = codegraph::project_root(&js_string(b.get("dir")))?;
            let rel = match b.get("path") {
                None | Some(Value::Null) => String::new(),
                v => js_string(v),
            };
            let file = node_path::resolve(&[&root, &rel]);
            if !file.starts_with(&root) || !std::path::Path::new(&file).exists() {
                return Ok(error_json(400, "File is not inside this project"));
            }
            // Windows needs a shell to run code.cmd: quote the path and refuse characters cmd.exe would interpret.
            if file.contains(['"', '%', '^', '&', '|', '<', '>']) {
                return Ok(error_json(400, "This file name can't be opened safely from here"));
            }
            let line = fjs::to_number(b.get("line"));
            let target = if line > 0.0 { format!("{file}:{}", fjs::num_str(line)) } else { file };
            if open_in_vscode(&target).await {
                Ok(json_response(200, &json!({"opened": target.replace('\\', "/")})))
            } else {
                Ok(error_json(
                    503,
                    "VS Code's \"code\" command was not found — in VS Code run: Shell Command: Install 'code' command in PATH",
                ))
            }
        }

        "POST /api/code/link" => {
            // Confirm a Figma component ↔ code component pair (writes wara.components.json in the project).
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_FIRST).await? {
                return Ok(r);
            }
            let dir = js_string(b.get("dir"));
            let figma_name = js_string(b.get("figmaName"));
            let symbol_id = fjs::to_number(b.get("symbolId"));
            let out = blocking(move || {
                let (db, _) = wg::open_graph(&dir)?;
                wg::design::confirm_link(&wg::query::graph(&db)?, &codegraph::project_root(&dir)?, &figma_name, symbol_id)
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        // ---- brain: a project's Markdown notes (brain/), their links, and the trash ----
        "GET /api/brain" | "GET /api/brain/code-links" | "GET /api/brain/note" => {
            let dir = match known_query_dir(app, req.param("dir"), ADD_BRAIN).await? {
                Ok(d) => d,
                Err(r) => return Ok(r),
            };
            let root = codegraph::project_root(&dir)?;
            let name = req.param("name").unwrap_or_default();
            let which = path.clone();
            let out = blocking(move || match which.as_str() {
                "/api/brain" => wg::brain::brain_graph(&root),
                "/api/brain/code-links" => Ok(Value::Array(wg::brain::code_links(&root)?)),
                _ => wg::brain::read_note(&root, &name),
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        "POST /api/brain/note" | "POST /api/brain/trash" | "POST /api/brain/restore" | "POST /api/brain/empty-trash" => {
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "dir")?;
            if let Some(r) = known_body_dir(app, b.get("dir"), ADD_BRAIN).await? {
                return Ok(r);
            }
            let root = codegraph::project_root(&js_string(b.get("dir")))?;
            let which = path.clone();
            // Permanent (empty-trash): only the page's own button (after the user confirms) calls it; no AI tool does.
            let out = blocking(move || match which.as_str() {
                "/api/brain/note" => brain_write(&root, &b),
                "/api/brain/trash" => wg::brain::trash_note(&root, &js_string(b.get("name"))),
                "/api/brain/restore" => wg::brain::restore_note(&root, codegraph::path_string("path", b.get("path"))?),
                _ => {
                    let p = b.get("path");
                    let one = if truthy(p) { Some(codegraph::path_string("path", p)?) } else { None };
                    wg::brain::empty_trash(&root, one)
                }
            })
            .await?;
            Ok(graph_json(200, &out))
        }

        // ---- PDF page: a section's frames as the pages of one document ----
        "GET /api/pdf/sections" => {
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            let doc = app.raw("get_document_info", json!({}), None).await?;
            let mut sections = vec![];
            for top in doc.get("topLevel").and_then(Value::as_array).cloned().unwrap_or_default() {
                if top.get("type") != Some(&json!("SECTION")) {
                    continue;
                }
                let Ok(node) = app.raw("get_node", obj! { "nodeId" => top.get("id").cloned(), "depth" => 1i64 }, None).await else {
                    continue; // section went away
                };
                let order = pdf::order_pages(prop(Some(&node), "children")?);
                if order.get("pages").and_then(Value::as_array).is_some_and(|p| !p.is_empty()) {
                    sections.push(obj! {
                        "id" => top.get("id").cloned(), "name" => top.get("name").cloned(),
                        "pages" => order.get("pages").cloned(), "warnings" => order.get("warnings").cloned(), "issues" => order.get("issues").cloned(),
                    });
                }
            }
            let mut page = file_page(&doc, "sections");
            page["sections"] = Value::Array(sections);
            Ok(js_json_response(200, &page))
        }

        "GET /api/pdf/download" => {
            // Built in memory and sent as a download; nothing is written on this machine.
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            // id=<one> or ids=<a,b,…>: several sections become one document, in the order given
            let raw = req.param("ids").or_else(|| req.param("id")).unwrap_or_default();
            let ids: Vec<Value> = raw.split(',').map(js_trim).filter(|s| !s.is_empty()).map(|s| Value::String(s.to_string())).collect();
            if ids.is_empty() {
                return Ok(error_json(400, "id or ids is required"));
            }
            let made = pdf::design_pdf(
                |command: String, params: Value, timeout: Option<u64>| app.raw(&command, params, timeout),
                &Value::Array(ids),
            )
            .await?;
            let name = pdf::pdf_file_name(made.title.map(Value::String).as_ref());
            // filename= takes printable ASCII only (one "_" per UTF-16 unit of anything else); filename* the whole name
            let ascii: String = name.encode_utf16().map(|u| if (0x20..=0x7e).contains(&u) { char::from(u as u8) } else { '_' }).collect();
            let disposition = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", crate::util::encode_uri_component(&name));
            Ok(Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CONTENT_LENGTH, made.pdf.len())
                .header(header::CONTENT_DISPOSITION, disposition)
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(made.pdf))
                .unwrap())
        }

        "GET /api/thumb" => {
            // Screen preview for the graph side panel; not recorded in the command log.
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            let Some(id) = req.param("id").filter(|s| !s.is_empty()) else {
                return Ok(error_json(400, "id is required"));
            };
            let r = app.raw("export_node_image", json!({"nodeId": id, "scale": 1, "maxSize": 640}), Some(60_000)).await?;
            let Some(b64) = r.get("base64").and_then(Value::as_str) else {
                return Err(anyhow!(
                    "The first argument must be of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object. Received undefined"
                ));
            };
            Ok(bytes_response("image/png", "no-store", crate::util::base64_lenient(b64)))
        }

        "GET /api/outline" => {
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected"));
            }
            let Some(id) = req.param("id").filter(|s| !s.is_empty()) else {
                return Ok(error_json(400, "id is required"));
            };
            let n = req.param("depth").map(|d| crate::util::js_number(&d)).unwrap_or(0.0);
            let n = if n == 0.0 || n.is_nan() { 3.0 } else { n };
            let depth = 6f64.min(1f64.max(n));
            let r = app
                .raw("frame_outline", json!({"nodeId": id, "depth": crate::jsjson::num(depth), "maxChildren": 40}), Some(60_000))
                .await?;
            Ok(json_response(200, &r))
        }

        "GET /api/graph" => {
            if !app.bridge.connected() {
                return Ok(error_json(503, "Figma plugin is not connected — run Plugins → Development → wridge"));
            }
            let page_id = req.param("pageId").filter(|s| !s.is_empty());
            let refresh = req.param("refresh").as_deref() == Some("1");
            let mut params = Map::new();
            if let Some(p) = &page_id {
                params.insert("pageId".into(), p.clone().into());
            }
            params.insert("refresh".into(), refresh.into());
            let map = app.raw("design_map", Value::Object(params), Some(120_000)).await?;
            let mut fparams = Map::new();
            if let Some(p) = &page_id {
                fparams.insert("pageId".into(), p.clone().into());
            }
            fparams.insert("limit".into(), 1.into());
            // Issue highlighting is optional; the graph still renders without it.
            let flows = app.raw("check_flows", Value::Object(fparams), Some(120_000)).await.ok();
            Ok(json_response(200, &crate::graph::to_graph_data(&map, flows.as_ref())))
        }

        "GET /guide" => Ok(Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(app.read_guide()))
            .unwrap()),

        // ---- skills page (read-only; works without Figma) ----
        "GET /api/skills" => {
            let list: Vec<Value> = app
                .load_skills()
                .into_iter()
                .map(|s| {
                    let area = if s.area == "design" || s.area == "code" { s.area.clone() } else { "both".into() }; // no area = listed in every session
                    json!({"name": s.name, "description": s.description, "area": area, "file": format!("skills/{}", s.file), "tokens": s.tokens, "body": s.body})
                })
                .collect();
            Ok(json_response(200, &Value::Array(list)))
        }

        "POST /api/skills/open" => {
            // Open one of the shipped skill files in VS Code. Only names from the skills list are accepted.
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "file")?;
            let file = b.get("file").and_then(Value::as_str).map(str::to_string);
            let skill = app
                .load_skills()
                .into_iter()
                .find(|s| file.as_deref() == Some(&format!("skills/{}", s.file)) || file.as_deref() == Some(&s.file));
            let Some(skill) = skill else { return Ok(error_json(404, "No such skill")) };
            let path = app.paths.skills.join(&skill.file).display().to_string();
            if path.contains(['"', '%', '^', '&', '|', '<', '>']) {
                return Ok(error_json(400, "This file name can't be opened safely from here"));
            }
            if open_in_vscode(&path).await {
                Ok(json_response(200, &json!({"opened": path.replace('\\', "/")})))
            } else {
                Ok(error_json(
                    503,
                    "VS Code's \"code\" command was not found — in VS Code run: Shell Command: Install 'code' command in PATH",
                ))
            }
        }

        "GET /api/status" => {
            app.bridge.refresh_file_name();
            let (today, history, started_at) = {
                let mut s = app.run.lock().unwrap();
                if s.today.day != crate::app::date_string() {
                    s.today = crate::app::Today::new();
                }
                (s.today.to_json(), Value::Array(s.history.iter().cloned().collect()), s.started_at)
            };
            Ok(json_response(
                200,
                &json!({
                    "version": VERSION,
                    "fileName": app.bridge.file_name(),
                    "openFiles": app.bridge.open_files(),
                    "today": today,
                    "coreToolCount": app.profile_tools("core").len(),
                    "skillCount": app.load_skills().len(),
                    "guidePath": slash(&app.paths.guide),
                    "cliPath": slash(&app.paths.cli),
                    "pid": std::process::id(),
                    "startedAt": started_at,
                    "paused": app.paused(),
                    "pluginConnected": app.bridge.plugin_connected(),
                    "ports": {"plugin": app.ws_port, "http": app.http_port},
                    "toolCount": app.tools.len(),
                    "history": history,
                }),
            ))
        }

        "GET /api/sessions" => Ok(json_response(200, &app.bridge.sessions_info())),

        // the wridge window picks the Figma file it shows (by file key, or name for older plugins)
        "POST /api/figma-file" => {
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "file")?;
            match app.bridge.use_file(b.get("file")) {
                Ok(_) => Ok(json_response(200, &app.bridge.sessions_info())),
                Err(e) => Ok(json_response(404, &json!({"error": e.to_string()}))),
            }
        }

        "GET /api/tools" => {
            let list: Vec<Value> = app
                .tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "params": tool_schema(t)}))
                .collect();
            Ok(json_response(200, &Value::Array(list)))
        }

        "POST /api/tool" => {
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "name")?;
            let name = b.get("name");
            let Some(t) = app.find_tool(name) else {
                return Ok(json_response(
                    404,
                    &json!({"isError": true, "content": [{"type": "text", "text": format!("Unknown tool \"{}\"", js_string(name))}]}),
                ));
            };
            let args = match b.get("args") {
                None | Some(Value::Null) => json!({}),
                Some(a) => a.clone(),
            };
            match zod::safe_parse(&t.shape, Some(&args)) {
                Err(issues) => Ok(json_response(
                    400,
                    &json!({"isError": true, "content": [{"type": "text", "text": zod::prettify_error(&issues)}]}),
                )),
                Ok(parsed) => {
                    // scripts and the CLI name their session with X-Wridge-Session (session.rs)
                    let session = crate::session::parse(req.header(crate::session::HEADER).as_deref()).unwrap_or_else(|| std::sync::Arc::from(crate::session::SHARED));
                    let result = crate::session::scope(session, app.run_tool(&t, parsed, "CLI")).await;
                    Ok(json_response(200, &result))
                }
            }
        }

        "POST /api/pause" => {
            let body = req.json(BODY_LIMIT).await?;
            let b = destructure(&body, "paused")?;
            let paused = truthy(b.get("paused"));
            app.bridge.set_paused(paused);
            log(if paused { "Paused AI commands" } else { "Resumed AI commands" });
            Ok(json_response(200, &json!({"paused": paused})))
        }

        "POST /api/restart" => {
            crate::server::restart(app.clone());
            Ok(json_response(200, &json!({"restarting": true})))
        }

        "POST /api/stop" => {
            log("Stopped from dashboard");
            crate::server::shutdown(app.clone(), 0);
            Ok(json_response(200, &json!({"stopping": true})))
        }

        _ => {
            let accept = req.header("accept").unwrap_or_default();
            if get && !path.starts_with("/api/") && accept.contains("text/html") {
                let bytes = read_file(&app.paths.web.join("404.html"))?;
                return Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(Body::from(String::from_utf8_lossy(&bytes).into_owned()))
                    .unwrap());
            }
            Ok(error_json(404, "not found"))
        }
    }
}

/// `code -g "<file>"` through the shell, like Node's spawn(..., { shell: true }). The launcher hands the file
/// to VS Code and exits; a non-zero exit means "code" isn't installed. No answer within 5 s counts as opened.
pub async fn open_in_vscode(target: &str) -> bool {
    #[cfg(windows)]
    let child = {
        let mut cmd = tokio::process::Command::new("cmd.exe");
        cmd.raw_arg(format!("/d /s /c \"code -g \"{target}\"\"")).creation_flags(crate::util::CREATE_NO_WINDOW);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    #[cfg(not(windows))]
    let child = tokio::process::Command::new("code")
        .args(["-g", target])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return false };
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) => false,
        Err(_) => true,
    }
}

/// `{ file: doc.fileName, page: doc.currentPage?.name, <list>: [] }` — undefined values left out, as JSON.stringify does.
fn file_page(doc: &Value, list: &str) -> Value {
    let mut out = Map::new();
    if let Some(f) = doc.get("fileName") {
        out.insert("file".into(), f.clone());
    }
    if let Some(p) = doc.get("currentPage").and_then(|p| p.get("name")) {
        out.insert("page".into(), p.clone());
    }
    out.insert(list.into(), json!([]));
    Value::Object(out)
}

// ---------- code graph routes (index.mjs) ----------

/// `result.content?.[0]?.text ?? ""`
fn first_text(result: &Value) -> Value {
    match result.get("content").and_then(|c| c.get(0)).and_then(|c| c.get("text")) {
        None | Some(Value::Null) => Value::String(String::new()),
        Some(v) => v.clone(),
    }
}

/// `/\.json$/i`
fn json_ext() -> regress::Regex {
    regress::Regex::with_flags(r"\.json$", "i").unwrap()
}

/// GET /api/motion's tokens: the motion-named design tokens the code defines (`exportGraph(db, root).tokens`).
fn motion_tokens(dir: &str, root: &str) -> anyhow::Result<Vec<Value>> {
    let (db, _) = wg::open_graph(dir)?;
    let graph = wg::export::export_graph(&db, root)?;
    let Some(tokens) = graph.get("tokens").and_then(Value::as_array) else {
        return Err(anyhow!("Cannot read properties of undefined (reading 'filter')"));
    };
    let motion = regress::Regex::with_flags("duration|easing|ease|motion|curve|transition", "i").unwrap();
    Ok(tokens
        .iter()
        .filter(|t| truthy(t.get("value")) && motion.find(&js_string(t.get("name"))).is_some())
        .map(|t| {
            let mut o = Map::new();
            let value = match t.get("resolved") {
                None | Some(Value::Null) => t.get("value"),
                r => r,
            };
            for (k, v) in [("name", t.get("name")), ("value", value), ("defined", t.get("defined")), ("count", t.get("count"))] {
                if let Some(v) = v {
                    o.insert(k.into(), v.clone());
                }
            }
            Value::Object(o)
        })
        .collect())
}

/// POST /api/motion/figma-to-lottie past the project check.
async fn figma_to_lottie(app: &Ctx, b: &Map<String, Value>) -> anyhow::Result<Response> {
    if !app.bridge.connected() {
        return Ok(error_json(503, "Figma plugin is not connected"));
    }
    let out = b.get("out");
    let out_text = match out {
        None | Some(Value::Null) => String::new(),
        v => js_string(v),
    };
    if !truthy(b.get("id")) || json_ext().find(&out_text).is_none() {
        return Ok(error_json(400, "id and out (….json) are required"));
    }
    let root = codegraph::project_root(&js_string(b.get("dir")))?;
    let out = codegraph::path_string("paths[1]", out)?.to_string();
    if !truthy(b.get("overwrite")) && std::path::Path::new(&node_path::resolve(&[&root, &out])).exists() {
        return Ok(json_response(409, &json!({"error": "exists", "path": out})));
    }
    let t = app.tool("motion_to_lottie").ok_or_else(|| anyhow!("Cannot read properties of undefined (reading 'run')"))?;
    let mut args = Map::new();
    args.insert("projectDir".into(), root.into());
    args.insert("out".into(), out.clone().into());
    if let Some(id) = b.get("id") {
        args.insert("nodeId".into(), id.clone());
    }
    let result = app.run_tool(&t, Value::Object(args), "motion").await;
    let text = first_text(&result);
    if truthy(result.get("isError")) {
        return Ok(json_response(400, &json!({"error": text})));
    }
    Ok(json_response(200, &json!({"path": out.replace('\\', "/"), "report": text})))
}

/// index.mjs `call(name, args)` in POST /api/motion/figma: a tool through runTool ("motion" in the log), its
/// text parsed as JSON when it is JSON; a tool error is thrown.
async fn call_json(app: &Ctx, name: &str, args: Value) -> anyhow::Result<Value> {
    let t = app.tool(name).ok_or_else(|| anyhow!("Cannot read properties of undefined (reading 'run')"))?;
    let out = app.run_tool(&t, args, "motion").await;
    let body = first_text(&out);
    if truthy(out.get("isError")) {
        return Err(anyhow!(js_string(Some(&body))));
    }
    Ok(match &body {
        Value::String(s) => crate::jsjson::parse(s).unwrap_or(body.clone()),
        _ => body,
    })
}

/// POST /api/motion/figma past the project and plugin checks.
async fn motion_to_figma(app: &Ctx, b: &Map<String, Value>) -> anyhow::Result<Response> {
    let doc = call_json(app, "get_document_info", json!({})).await?;
    let file_name = prop(Some(&doc), "fileName")?.cloned();
    let confirm = b.get("confirmFile");
    if !truthy(confirm) || !fjs::strict_eq(confirm, file_name.as_ref()) {
        let mut o = Map::new();
        if let Some(f) = &file_name {
            o.insert("file".into(), f.clone());
        }
        let error = if truthy(confirm) { "Another Figma file is open now" } else { "Confirm the Figma file first" };
        o.insert("error".into(), error.into());
        return Ok(graph_json(409, &Value::Object(o)));
    }
    let root = codegraph::project_root(&js_string(b.get("dir")))?;
    let p = codegraph::path_string("paths[1]", b.get("path"))?.to_string();
    let (r, pp) = (root.clone(), p.clone());
    let anim = blocking(move || motion::read_lottie(&r, &pp)).await?;
    let last = p.rsplit('/').next().unwrap_or("");
    let stem = match json_ext().find(last) {
        Some(m) => format!("{}{}", &last[..m.start()], &last[m.end()..]),
        None => last.to_string(),
    };
    let name = format!("Lottie/{stem}");
    let gif = match b.get("gif") {
        None | Some(Value::Null) => String::new(),
        v => js_string(v),
    };
    let gif_path = std::env::temp_dir().join(format!("wara-{}.gif", uuid::Uuid::new_v4()));
    std::fs::write(&gif_path, crate::util::base64_lenient(&gif))?;
    let gif_file = gif_path.to_string_lossy().into_owned();
    let result = place_gif(app, &doc, &anim, (&root, &p, &name, &gif_file), file_name).await;
    let _ = std::fs::remove_file(&gif_path); // already gone: fine
    result
}

/// The GIF into the same Lottie component again (in this file), else a new one next to the art it was made
/// from; then the Lottie file remembers the component.
async fn place_gif(app: &Ctx, doc: &Value, anim: &Value, (root, p, name, gif): (&str, &str, &str, &str), file_name: Option<Value>) -> anyhow::Result<Response> {
    let a = Some(anim);
    let wara = fjs::get(fjs::get(a, "meta"), "wara");
    let known = fjs::get(wara, "figma");
    // componentId: None = undefined
    let mut component_id: Option<Value> = Some(Value::Null);
    if truthy(fjs::get(known, "componentId")) && fjs::strict_eq(fjs::get(known, "file"), file_name.as_ref()) {
        let mut args = Map::new();
        args.insert("nodeId".into(), fjs::get(known, "componentId").cloned().unwrap_or(Value::Null));
        args.insert("depth".into(), 0.into());
        // deleted in Figma: a new one
        if let Ok(n) = call_json(app, "get_node", Value::Object(args)).await {
            if !n.is_null() && fjs::get(Some(&n), "type") == Some(&json!("COMPONENT")) {
                component_id = fjs::get(Some(&n), "id").cloned();
            }
        }
    }
    let mut created = false;
    let fill = |node: Option<Value>| {
        let mut args = Map::new();
        if let Some(n) = node {
            args.insert("nodeId".into(), n);
        }
        args.insert("source".into(), gif.into());
        args.insert("scaleMode".into(), "FIT".into());
        Value::Object(args)
    };
    if !truthy(component_id.as_ref()) {
        // Next to everything on the page unless the art it came from is known (then right of the art).
        let top_level: Vec<Value> = match fjs::nullish(fjs::get(Some(doc), "topLevel")) {
            None => vec![],
            Some(Value::Array(t)) => t.clone(),
            Some(_) => return Err(anyhow!("(doc.topLevel ?? []).map is not a function")),
        };
        let right = |n: &Value| fjs::to_number(n.get("x")) + fjs::to_number(n.get("width"));
        let mut x = fjs::max_of(std::iter::once(0.0).chain(top_level.iter().map(right))) + 200.0;
        let mut y = Some(json!(0));
        let source = fjs::get(fjs::get(wara, "spec"), "nodeId");
        // get_node gives x/y in the parent's space: only a top-level art frame says where it is on the page.
        if let Some(top) = top_level.iter().find(|n| fjs::strict_eq(n.get("id"), source)) {
            x = right(top) + 80.0;
            y = top.get("y").cloned();
        }
        let mut args = Map::new();
        args.insert("name".into(), name.into());
        for (k, v) in [("width", anim.get("w")), ("height", anim.get("h"))] {
            if let Some(v) = v {
                args.insert(k.into(), v.clone());
            }
        }
        args.insert("layoutMode".into(), "NONE".into());
        args.insert("x".into(), fjs::num(x));
        if let Some(y) = y {
            args.insert("y".into(), y);
        }
        let frame = call_json(app, "create_frame", Value::Object(args)).await?;
        let frame_id = prop(Some(&frame), "id")?.cloned();
        call_json(app, "set_image_fill", fill(frame_id.clone())).await?;
        let mut args = Map::new();
        if let Some(id) = frame_id {
            args.insert("nodeId".into(), id);
        }
        args.insert("name".into(), name.into());
        args.insert(
            "description".into(),
            format!("lottie: {p}\nGIF preview for Figma (moves in Present). The app plays the .json file.").into(),
        );
        let comp = call_json(app, "create_component", Value::Object(args)).await?;
        component_id = prop(Some(&comp), "id")?.cloned();
        created = true;
    } else {
        call_json(app, "set_image_fill", fill(component_id.clone())).await?;
    }
    let mut info = Map::new();
    if let Some(c) = &component_id {
        info.insert("componentId".into(), c.clone());
    }
    if let Some(f) = &file_name {
        info.insert("file".into(), f.clone());
    }
    let (r, pp) = (root.to_string(), p.to_string());
    let file = blocking(move || motion::mark_sent_to_figma(&r, &pp, &Value::Object(info))).await?;
    let mut out = Map::new();
    out.insert("ok".into(), true.into());
    out.insert("created".into(), created.into());
    if let Some(c) = component_id {
        out.insert("componentId".into(), c);
    }
    if let Some(f) = file_name {
        out.insert("figmaFile".into(), f);
    }
    out.insert("file".into(), file);
    Ok(graph_json(200, &Value::Object(out)))
}

/// PUT /api/flows past the project check: writes docs/flows/<name>.md unless it changed since it was opened.
async fn save_flow(b: &Map<String, Value>) -> anyhow::Result<Response> {
    let b = b.clone();
    let saved = blocking(move || -> anyhow::Result<Result<Value, wg::flows::FlowConflict>> {
        let dir = js_string(b.get("dir"));
        let root = codegraph::project_root(&dir)?;
        let flow = fjs::nullish(b.get("flow"));
        let text = match fjs::nullish(b.get("content")) {
            None => wg::flows::format_flow(flow.unwrap_or(&json!({})))?,
            Some(Value::String(s)) => s.clone(),
            Some(other) => {
                return Err(anyhow!(
                    "The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received {}",
                    codegraph::received(Some(other))
                ))
            }
        };
        let name = match fjs::nullish(b.get("file")) {
            Some(f) => js_string(Some(f)),
            None => {
                let title = match fjs::nullish(fjs::get(flow, "title")) {
                    Some(t) => js_string(Some(t)),
                    None => js_string(wg::flows::parse_flow(&text).get("title")),
                };
                format!("{}.md", wg::flows::slugify(&title))
            }
        };
        let base = fjs::nullish(b.get("baseHash")).cloned().unwrap_or(Value::Null);
        match wg::flows::save_flow_file(&root, &name, &text, &base) {
            Ok(saved) => {
                let (db, _) = wg::open_graph(&dir)?;
                let parsed = wg::flows::parse_flow(&text);
                let check = wg::flows::check_flow(&wg::query::graph(&db)?, &parsed);
                let mut out = saved.as_object().cloned().unwrap_or_default();
                if let Value::Object(p) = parsed {
                    for (k, v) in p {
                        out.insert(k, v);
                    }
                }
                out.insert("check".into(), check);
                Ok(Ok(Value::Object(out)))
            }
            Err(err) => match err.downcast_ref::<wg::flows::FlowConflict>() {
                Some(c) => Ok(Err(c.clone())),
                None => Err(err),
            },
        }
    })
    .await?;
    match saved {
        Ok(v) => Ok(graph_json(200, &v)),
        Err(conflict) => {
            let hash = conflict.current.as_deref().map(wg::flows::hash_text);
            Ok(graph_json(409, &json!({"error": conflict.to_string(), "current": conflict.current, "currentHash": hash})))
        }
    }
}

/// POST /api/brain/note: the page's editor saves in place (`keepOld: false`). `mode` as given: undefined is
/// "create", and a value that is no mode (null, a number) overwrites and comes back as it was.
fn brain_write(root: &str, b: &Map<String, Value>) -> anyhow::Result<Value> {
    const OTHER: &str = "\u{0}other";
    let text = match b.get("content") {
        None | Some(Value::Null) => None,
        v => Some(js_string(v)),
    };
    let given = b.get("mode");
    let mode = match given {
        None => "create".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => OTHER.to_string(),
    };
    let opts = wg::brain::WriteOptions {
        text,
        mode,
        tags: None,
        keep_old: false,
    };
    let mut out = wg::brain::write_note(root, &js_string(b.get("name")), &opts)?;
    if out.get("mode").and_then(Value::as_str) == Some(OTHER) {
        out["mode"] = given.cloned().unwrap_or(Value::Null);
    }
    Ok(out)
}
