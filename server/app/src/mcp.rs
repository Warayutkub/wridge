//! MCP over Streamable HTTP at /mcp?profile=…, answering exactly like the Node server (MCP TypeScript SDK
//! 1.31, stateless StreamableHTTPServerTransport + McpServer): the same JSON-RPC envelopes, SSE framing,
//! status codes, error texts, tool list (with `execution.taskSupport`), prompts and instructions.
//!
//! Written against the SDK's behaviour rather than on rmcp: rmcp's `Tool` has no `execution` field and its
//! version negotiation and error texts differ, and the contract here is what Node answers.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method};
use axum::response::Response;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;

use crate::app::{Ctx, VERSION};
use crate::http::Req;
use crate::jsjson::{stringify, stringify_pretty};
use crate::session::{self, SessionId};
use crate::tools::Tool;
use crate::zod::{self, mcp_error_message, DRAFT_07};

pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_BATCH_SIZE: usize = 100;
const KEEP_ALIVE: Duration = Duration::from_millis(15_000);

fn error_response(status: u16, code: i64, message: &str) -> Response {
    let body = json!({"jsonrpc": "2.0", "error": {"code": code, "message": message}, "id": null});
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(stringify(&body)))
        .unwrap()
}

fn header_str(req: &Req, name: &str) -> Option<String> {
    req.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

fn protocol_error(req: &Req) -> Option<Response> {
    let v = header_str(req, "mcp-protocol-version")?;
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&v.as_str()) {
        return None;
    }
    Some(error_response(
        400,
        -32000,
        &format!(
            "Bad Request: Unsupported protocol version: {v} (supported versions: {})",
            SUPPORTED_PROTOCOL_VERSIONS.join(", ")
        ),
    ))
}

fn sse_response(body: Body) -> Response {
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .unwrap()
}

pub async fn handle(app: &Ctx, req: Req, profile: &str) -> Response {
    match req.method {
        Method::POST => post(app, req, profile).await,
        Method::GET => {
            let accept = header_str(&req, "accept").unwrap_or_default();
            if !accept.contains("text/event-stream") {
                return error_response(406, -32000, "Not Acceptable: Client must accept text/event-stream");
            }
            if let Some(r) = protocol_error(&req) {
                return r;
            }
            // A stateless server never pushes anything here: the stream only carries keep-alives.
            let stream = futures::stream::unfold((), |_| async {
                tokio::time::sleep(KEEP_ALIVE).await;
                Some((Ok::<_, Infallible>(axum::body::Bytes::from_static(b": keepalive\n\n")), ()))
            });
            sse_response(Body::from_stream(stream))
        }
        Method::DELETE => {
            if let Some(r) = protocol_error(&req) {
                return r;
            }
            if let Some(id) = session::parse(header_str(&req, "mcp-session-id").as_deref()) {
                app.bridge.forget_session(&id);
            }
            Response::builder().status(200).body(Body::empty()).unwrap()
        }
        _ => {
            let body = json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "Method not allowed."}, "id": null});
            Response::builder()
                .status(405)
                .header(header::ALLOW, "GET, POST, DELETE")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(stringify(&body)))
                .unwrap()
        }
    }
}

fn is_json_content_type(ct: Option<&str>) -> bool {
    let Some(ct) = ct else { return false };
    ct == "application/json" || ct.split(';').next().map(|s| s.trim().to_ascii_lowercase()) == Some("application/json".into())
}

fn is_request_id(v: &Value) -> bool {
    match v {
        Value::String(_) => true,
        Value::Number(n) => {
            let x = crate::jsjson::as_f64(n);
            x.fract() == 0.0 && x.abs() <= crate::jsjson::MAX_SAFE_INTEGER
        }
        _ => false,
    }
}

fn keys_within(o: &Map<String, Value>, allowed: &[&str]) -> bool {
    o.keys().all(|k| allowed.contains(&k.as_str()))
}

fn params_ok(o: &Map<String, Value>) -> bool {
    matches!(o.get("params"), None | Some(Value::Object(_)))
}

fn is_request(m: &Value) -> bool {
    let Value::Object(o) = m else { return false };
    keys_within(o, &["jsonrpc", "id", "method", "params"])
        && o.get("jsonrpc") == Some(&json!("2.0"))
        && o.get("id").map(is_request_id).unwrap_or(false)
        && o.get("method").map(Value::is_string).unwrap_or(false)
        && params_ok(o)
}

fn is_notification(m: &Value) -> bool {
    let Value::Object(o) = m else { return false };
    keys_within(o, &["jsonrpc", "method", "params"]) && o.get("jsonrpc") == Some(&json!("2.0")) && o.get("method").map(Value::is_string).unwrap_or(false) && params_ok(o)
}

fn is_response(m: &Value) -> bool {
    let Value::Object(o) = m else { return false };
    if o.get("jsonrpc") != Some(&json!("2.0")) {
        return false;
    }
    let result = keys_within(o, &["jsonrpc", "id", "result"]) && o.get("id").map(is_request_id).unwrap_or(false) && matches!(o.get("result"), Some(Value::Object(_)));
    let error = keys_within(o, &["jsonrpc", "id", "error"])
        && o.get("id").map(is_request_id).unwrap_or(true)
        && match o.get("error") {
            Some(Value::Object(e)) => e.get("code").and_then(Value::as_f64).map(|c| c.fract() == 0.0).unwrap_or(false) && e.get("message").map(Value::is_string).unwrap_or(false),
            _ => false,
        };
    result || error
}

/// One of the zod issues the SDK's request schemas raise (invalid_type only; that is all the
/// envelopes here can fail with).
struct ReqIssue {
    expected: &'static str,
    path: Vec<Value>,
    received: &'static str,
}

fn issues_message(issues: &[ReqIssue]) -> String {
    let list: Vec<Value> = issues
        .iter()
        .map(|i| json!({"expected": i.expected, "code": "invalid_type", "path": i.path, "message": format!("Invalid input: expected {}, received {}", i.expected, i.received)}))
        .collect();
    stringify_pretty(&Value::Array(list), 2)
}

fn check(issues: &mut Vec<ReqIssue>, v: Option<&Value>, expected: &'static str, path: &[&str]) -> bool {
    let ok = matches!((expected, v), ("string", Some(Value::String(_))) | ("object" | "record", Some(Value::Object(_))));
    if !ok {
        issues.push(ReqIssue {
            expected,
            path: path.iter().map(|p| json!(p)).collect(),
            received: zod::parsed_type(v),
        });
    }
    ok
}

/// InitializeRequestSchema: params { protocolVersion, capabilities, clientInfo { name, version } }.
fn initialize_issues(params: Option<&Value>) -> Vec<ReqIssue> {
    let mut issues = vec![];
    if !check(&mut issues, params, "object", &["params"]) {
        return issues;
    }
    let p = params.unwrap();
    check(&mut issues, p.get("protocolVersion"), "string", &["params", "protocolVersion"]);
    check(&mut issues, p.get("capabilities"), "object", &["params", "capabilities"]);
    if check(&mut issues, p.get("clientInfo"), "object", &["params", "clientInfo"]) {
        let c = p.get("clientInfo").unwrap();
        check(&mut issues, c.get("name"), "string", &["params", "clientInfo", "name"]);
        check(&mut issues, c.get("version"), "string", &["params", "clientInfo", "version"]);
    }
    issues
}

fn is_initialize_request(m: &Value) -> bool {
    m.get("method") == Some(&json!("initialize")) && initialize_issues(m.get("params")).is_empty()
}

async fn post(app: &Ctx, mut req: Req, profile: &str) -> Response {
    let accept = header_str(&req, "accept").unwrap_or_default();
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return error_response(406, -32000, "Not Acceptable: Client must accept both application/json and text/event-stream");
    }
    if !is_json_content_type(header_str(&req, "content-type").as_deref()) {
        return error_response(415, -32000, "Unsupported Media Type: Content-Type must be application/json");
    }
    let too_large = || error_response(413, -32000, &format!("Payload Too Large: Request body must not exceed {MAX_BODY} bytes"));
    if header_str(&req, "content-length")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|n| n > MAX_BODY as f64)
        .unwrap_or(false)
    {
        return too_large();
    }
    let body = std::mem::replace(&mut req.body, Body::empty());
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else { return too_large() };
    let Ok(raw) = serde_json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) else {
        return error_response(400, -32700, "Parse error: Invalid JSON");
    };
    if let Value::Array(a) = &raw {
        if a.len() > MAX_BATCH_SIZE {
            return error_response(400, -32600, &format!("Invalid Request: Batch must not exceed {MAX_BATCH_SIZE} messages"));
        }
    }
    let messages: Vec<Value> = match raw {
        Value::Array(a) => a,
        other => vec![other],
    };
    if !messages.iter().all(|m| is_request(m) || is_notification(m) || is_response(m)) {
        return error_response(400, -32700, "Parse error: Invalid JSON-RPC message");
    }
    let init = messages.iter().any(is_initialize_request);
    if init && messages.len() > 1 {
        return error_response(400, -32600, "Invalid Request: Only one initialization request is allowed");
    }
    if !init {
        if let Some(r) = protocol_error(&req) {
            return r;
        }
    }
    let requests: Vec<Value> = messages.into_iter().filter(is_request).collect();
    if requests.is_empty() {
        // only notifications or responses
        return Response::builder().status(202).body(Body::empty()).unwrap();
    }
    // Who is calling (session.rs): a new id on initialize, else the one the client echoes back. An id this
    // server never gave out (a restart) is used as-is rather than refused.
    let session_id: SessionId = if init {
        Arc::from(uuid::Uuid::new_v4().to_string())
    } else {
        session::parse(header_str(&req, "mcp-session-id").as_deref()).unwrap_or_else(|| Arc::from(session::SHARED))
    };

    let (tx, rx) = mpsc::unbounded_channel::<Result<axum::body::Bytes, Infallible>>();
    let app = app.clone();
    let profile = profile.to_string();
    let scoped = session_id.clone();
    tokio::spawn(session::scope(scoped, async move {
        let mut pending: FuturesUnordered<_> = requests.into_iter().map(|m| respond(app.clone(), profile.clone(), m)).collect();
        let mut keep_alive = tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
        loop {
            tokio::select! {
                next = pending.next() => match next {
                    Some(response) => {
                        let _ = tx.send(Ok(format!("event: message\ndata: {}\n\n", stringify(&response)).into()));
                    }
                    None => break,
                },
                _ = keep_alive.tick() => {
                    let _ = tx.send(Ok(axum::body::Bytes::from_static(b": keepalive\n\n")));
                }
            }
        }
    }));
    let mut response = sse_response(Body::from_stream(tokio_stream::wrappers::UnboundedReceiverStream::new(rx)));
    if init {
        if let Ok(v) = axum::http::HeaderValue::from_str(&session_id) {
            response.headers_mut().insert("mcp-session-id", v);
        }
    }
    response
}

fn success(id: &Value, result: Value) -> Value {
    json!({"result": result, "jsonrpc": "2.0", "id": id})
}

fn failure(id: &Value, code: i64, message: String) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// `new McpError(code, message).message`
fn mcp_message(code: i64, message: &str) -> String {
    format!("MCP error {code}: {message}")
}

fn tool_error(message: String) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

pub fn list_tools(tools: &[Arc<Tool>]) -> Value {
    let list: Vec<Value> = tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": zod::to_json_schema(&t.shape, DRAFT_07), "execution": {"taskSupport": "forbidden"}}))
        .collect();
    json!({ "tools": list })
}

async fn respond(app: Ctx, profile: String, m: Value) -> Value {
    let id = m.get("id").cloned().unwrap_or(Value::Null);
    let method = m.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let params = m.get("params");
    match method.as_str() {
        "initialize" => {
            let issues = initialize_issues(params);
            if !issues.is_empty() {
                return failure(&id, -32603, issues_message(&issues));
            }
            let requested = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str).unwrap_or("");
            let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
                requested
            } else {
                LATEST_PROTOCOL_VERSION
            };
            let tools = app.profile_tools(&profile);
            let instructions = app.instructions(&tools, &profile);
            let mut result = Map::new();
            result.insert("protocolVersion".into(), version.into());
            result.insert("capabilities".into(), json!({"tools": {"listChanged": true}, "prompts": {"listChanged": true}}));
            result.insert("serverInfo".into(), json!({"name": "wridge", "version": VERSION}));
            if !instructions.is_empty() {
                result.insert("instructions".into(), instructions.into());
            }
            success(&id, Value::Object(result))
        }
        "ping" => success(&id, json!({})),
        "tools/list" => success(&id, list_tools(&app.profile_tools(&profile))),
        "tools/call" => {
            let mut issues = vec![];
            if check(&mut issues, params, "object", &["params"]) {
                let p = params.unwrap();
                check(&mut issues, p.get("name"), "string", &["params", "name"]);
                if p.get("arguments").is_some() {
                    check(&mut issues, p.get("arguments"), "record", &["params", "arguments"]);
                }
            }
            if !issues.is_empty() {
                return failure(&id, -32603, issues_message(&issues));
            }
            let p = params.unwrap();
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            let tools = app.profile_tools(&profile);
            let Some(tool) = tools.iter().find(|t| t.name == name).cloned() else {
                return success(&id, tool_error(mcp_message(-32602, &format!("Tool {name} not found"))));
            };
            match zod::safe_parse(&tool.shape, p.get("arguments")) {
                Err(issues) => success(
                    &id,
                    tool_error(mcp_message(
                        -32602,
                        &format!("Input validation error: Invalid arguments for tool {name}: {}", mcp_error_message(&issues)),
                    )),
                ),
                Ok(args) => success(&id, app.run_tool(&tool, args, "MCP").await),
            }
        }
        "prompts/list" => {
            // Each skill is also a prompt, so users can start it explicitly with /<skill> in their client.
            let prompts: Vec<Value> = app
                .load_skills()
                .into_iter()
                .map(|s| json!({"name": s.name, "description": s.description, "arguments": [{"name": "task", "required": false}]}))
                .collect();
            success(&id, json!({ "prompts": prompts }))
        }
        "prompts/get" => {
            let mut issues = vec![];
            if check(&mut issues, params, "object", &["params"]) {
                let p = params.unwrap();
                check(&mut issues, p.get("name"), "string", &["params", "name"]);
                if let Some(a) = p.get("arguments") {
                    if check(&mut issues, Some(a), "record", &["params", "arguments"]) {
                        for (k, v) in a.as_object().unwrap() {
                            if !v.is_string() {
                                issues.push(ReqIssue {
                                    expected: "string",
                                    path: vec![json!("params"), json!("arguments"), json!(k)],
                                    received: zod::parsed_type(Some(v)),
                                });
                            }
                        }
                    }
                }
            }
            if !issues.is_empty() {
                return failure(&id, -32603, issues_message(&issues));
            }
            let p = params.unwrap();
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(skill) = app.load_skills().into_iter().find(|s| s.name == name) else {
                return failure(&id, -32602, mcp_message(-32602, &format!("Prompt {name} not found")));
            };
            let Some(args) = p.get("arguments") else {
                return failure(
                    &id,
                    -32602,
                    mcp_message(-32602, &format!("Invalid arguments for prompt {name}: Invalid input: expected object, received undefined")),
                );
            };
            let task = args
                .get("task")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .unwrap_or("Ask me what to work on, then follow this skill.");
            let text = format!("{}\n\n---\nTask: {task}", skill.body);
            success(&id, json!({"messages": [{"role": "user", "content": {"type": "text", "text": text}}]}))
        }
        _ => failure(&id, -32601, "Method not found".into()),
    }
}
