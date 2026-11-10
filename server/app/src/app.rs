//! The tool side of index.mjs: the registry (tools.mjs + use_file + the meta tools), runTool and the
//! command history, profiles, and the MCP instructions text.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use crate::bridge::{Bridge, RawSend};
use crate::guards::{now_ms, FileGuard, PluginSend};
use crate::jsjson::{js_string, slice_utf16, stringify, truthy};
use crate::paths::Paths;
use crate::skills::{load_skills, skill_index, Skill};
use crate::tools::{define_tools, handler, text_result, Tool};
use crate::zod::{self, *};

pub type Ctx = Arc<App>;

pub const VERSION: &str = "1.8.3";

pub struct Today {
    pub day: String,
    pub count: u64,
    pub errors: u64,
    pub total_ms: u64,
}

impl Today {
    pub fn new() -> Today {
        Today {
            day: date_string(),
            count: 0,
            errors: 0,
            total_ms: 0,
        }
    }
    pub fn to_json(&self) -> Value {
        json!({"day": self.day, "count": self.count, "errors": self.errors, "totalMs": self.total_ms})
    }
}

impl Default for Today {
    fn default() -> Self {
        Today::new()
    }
}

/// `new Date().toDateString()`, e.g. "Fri Nov 06 2026".
pub fn date_string() -> String {
    chrono::Local::now().format("%a %b %d %Y").to_string()
}

pub struct RunState {
    pub started_at: u64,
    pub history: VecDeque<Value>,
    pub today: Today,
}

pub struct App {
    pub paths: Paths,
    pub ws_port: u16,
    pub http_port: u16,
    pub bridge: Arc<Bridge>,
    /// Figma writes only reach the file the AI checked (guards.rs); plugin-side guards live in code.ts.
    pub guarded: FileGuard,
    pub tools: Vec<Arc<Tool>>,
    /// read-only GitHub (github.rs); its poller starts in server::run
    pub github: Arc<crate::github::GitHub>,
    pub meta: HashSet<String>,
    pub run: Mutex<RunState>,
    /// Set by stop / restart (index.mjs shutdown()); the server loop watches it.
    pub shutdown: tokio::sync::watch::Sender<bool>,
}

const META_TOOLS: &[&str] = &["run_batch", "call_tool", "describe_tool", "get_skill"];

impl App {
    pub fn new(paths: Paths, ws_port: u16, http_port: u16) -> Ctx {
        let bridge = Bridge::new(http_port);
        let guarded = FileGuard::new(Arc::new(RawSend(bridge.clone())));
        let mut tools: Vec<Arc<Tool>> = define_tools().into_iter().map(Arc::new).collect();
        tools.push(Arc::new(use_file_tool()));
        for t in [batch_tool(), call_tool(), describe_tool(), get_skill_tool()] {
            tools.push(Arc::new(t));
        }
        Arc::new(App {
            paths,
            ws_port,
            http_port,
            bridge,
            guarded,
            tools,
            github: crate::github::GitHub::from_env(),
            meta: META_TOOLS.iter().map(|s| s.to_string()).collect(),
            run: Mutex::new(RunState {
                started_at: now_ms() as u64,
                history: VecDeque::new(),
                today: Today::new(),
            }),
            shutdown: tokio::sync::watch::channel(false).0,
        })
    }

    /// The guarded `send` the tools use.
    pub fn figma(&self, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
        self.guarded.send(command, params, timeout_ms)
    }

    /// The raw `send` the HTTP routes use (no file guard).
    pub fn raw(&self, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
        self.bridge.send(command, params, timeout_ms)
    }

    pub fn tool(&self, name: &str) -> Option<Arc<Tool>> {
        self.tools.iter().find(|t| t.name == name).cloned()
    }

    /// Some MCP clients hand the model prefixed names (e.g. "mcp_wara-bridge_create_frame").
    pub fn find_tool(&self, name: Option<&Value>) -> Option<Arc<Tool>> {
        if let Some(Value::String(s)) = name {
            if let Some(t) = self.tool(s) {
                return Some(t);
            }
        }
        let s = js_string(name);
        let re = regress::Regex::with_flags(r"^.*?wara[-_]bridge[_:.\-/]+", "i").unwrap();
        let stripped = match re.find(&s) {
            Some(m) => format!("{}{}", &s[..m.start()], &s[m.end()..]),
            None => s,
        };
        self.tool(&stripped)
    }

    pub fn is_meta(&self, name: &str) -> bool {
        self.meta.contains(name)
    }

    pub fn load_skills(&self) -> Vec<Skill> {
        load_skills(&self.paths.skills)
    }

    /// Read on every use so edits to the guide apply without a restart.
    pub fn read_guide(&self) -> String {
        match std::fs::read(&self.paths.guide) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(_) => "AI_GUIDE.md is missing from the wridge folder.".into(),
        }
    }

    pub fn paused(&self) -> bool {
        self.bridge.paused()
    }

    pub fn record(&self, entry: Value, ok: bool, ms: u64) {
        let mut s = self.run.lock().unwrap();
        s.history.push_front(entry);
        s.history.truncate(50);
        if s.today.day != date_string() {
            s.today = Today::new();
        }
        s.today.count += 1;
        s.today.total_ms += ms;
        if !ok {
            s.today.errors += 1;
        }
    }

    pub async fn run_tool(self: &Arc<Self>, t: &Arc<Tool>, args: Value, source: &str) -> Value {
        let started = now_ms() as u64;
        self.bridge.note_tool_call(&crate::session::current());
        let outcome = if self.paused() {
            Err(anyhow!("wridge is paused — resume it at http://localhost:{}", self.http_port))
        } else {
            (t.run)(self.clone(), args).await
        };
        let ms = (now_ms() as u64).saturating_sub(started);
        let mut entry = Map::new();
        entry.insert("time".into(), started.into());
        entry.insert("source".into(), source.into());
        entry.insert("tool".into(), t.name.clone().into());
        match outcome {
            Ok(result) => {
                // Tools like run_batch report failure in the result instead of throwing.
                let is_error = truthy(result.get("isError"));
                entry.insert("ok".into(), (!is_error).into());
                entry.insert("ms".into(), ms.into());
                if is_error {
                    let text = result.get("content").and_then(|c| c.get(0)).and_then(|c| c.get("text"));
                    let text = match text {
                        None | Some(Value::Null) => "failed".to_string(),
                        Some(v) => js_string(Some(v)),
                    };
                    entry.insert("error".into(), slice_utf16(&text, 500).into());
                }
                self.record(Value::Object(entry), !is_error, ms);
                result
            }
            Err(err) => {
                let msg = err.to_string();
                entry.insert("ok".into(), false.into());
                entry.insert("ms".into(), ms.into());
                entry.insert("error".into(), msg.clone().into());
                self.record(Value::Object(entry), false, ms);
                json!({"content": [{"type": "text", "text": msg}], "isError": true})
            }
        }
    }

    /// Tool schemas are the largest fixed cost per request, so each profile advertises only what its kind
    /// of work needs; every other tool stays reachable through call_tool and run_batch.
    pub fn profile_tools(&self, profile: &str) -> Vec<Arc<Tool>> {
        if profile == "all" {
            return self.tools.clone();
        }
        if profile == "design" {
            let core: HashSet<&str> = profile_names("core").into_iter().collect();
            let elsewhere: HashSet<&str> = profile_names("code").into_iter().chain(profile_names("prototype")).filter(|n| !core.contains(n)).collect();
            return self.tools.iter().filter(|t| !elsewhere.contains(t.name.as_str())).cloned().collect();
        }
        let names: HashSet<&str> = profile_names(profile).into_iter().chain(META_TOOLS.iter().copied()).collect();
        self.tools.iter().filter(|t| names.contains(t.name.as_str())).cloned().collect()
    }

    pub fn instructions(&self, tools: &[Arc<Tool>], profile: &str) -> String {
        let shown: HashSet<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let hidden: Vec<&str> = self.tools.iter().map(|t| t.name.as_str()).filter(|n| !shown.contains(n)).collect();
        let all = self.load_skills();
        let area = profile_area(profile);
        let skills: Vec<Skill> = match area {
            Some(a) => all.iter().filter(|s| s.area.is_empty() || s.area == "both" || s.area == a).cloned().collect(),
            None => all.clone(),
        };
        let others = all.len() - skills.len();
        let more = if hidden.is_empty() {
            String::new()
        } else {
            format!(
                "## More tools\nNot listed here but callable with call_tool / run_batch (describe_tool shows parameters): {}",
                hidden.join(", ")
            )
        };
        let skill_text = if skills.is_empty() {
            String::new()
        } else {
            let tail = if others > 0 {
                format!(
                    "\n\n{others} more skills ({} work) — their names are in the guide above; get_skill() lists them all.",
                    if area == Some("design") { "code" } else { "design" }
                )
            } else {
                String::new()
            };
            format!(
                "## Skills\nLoad the matching skill with get_skill(name) before starting that kind of work:\n{}{tail}",
                skill_index(&skills)
            )
        };
        [self.read_guide(), more, skill_text].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n")
    }
}

const DISCOVER: &[&str] = &[
    "design_map",
    "get_document_info",
    "use_file",
    "get_selection",
    "get_node",
    "find_nodes",
    "export_node_image",
];

/// PROFILES[profile] ?? PROFILES.core
pub fn profile_names(profile: &str) -> Vec<&'static str> {
    let with = |rest: &[&'static str]| DISCOVER.iter().copied().chain(rest.iter().copied()).collect::<Vec<_>>();
    match profile {
        "code" => with(&[
            "code_graph",
            "code_graph_update",
            "code_graph_note",
            "create_lottie",
            "motion_read",
            "motion_animate",
            "motion_to_lottie",
            "motion_check",
            "motion_tokens",
            "motion_to_code",
            "code_design_sync",
            "project_knowledge",
            "project_memory",
            "tasks",
            "report",
            "component_usage",
            "get_design_context",
            "get_tokens",
            "export_assets",
            "get_component_map",
            "map_component",
            "get_prototype",
            "get_components",
            "get_variables",
            "get_styles",
            "scan_design_values",
        ]),
        // FigJam boards: system design with the code and data graphs at hand.
        "figjam" => vec![
            "get_document_info",
            "use_file",
            "figjam_diagram",
            "board_map",
            "create_sticky",
            "create_shape",
            "create_connector",
            "create_text",
            "create_section",
            "move_resize",
            "set_text",
            "delete_node",
            "code_graph",
            "project_memory",
        ],
        "prototype" => with(&[
            "check_flows",
            "get_prototype",
            "add_interaction",
            "remove_interactions",
            "set_flow_start",
            "set_scroll",
            "select_nodes",
            "project_memory",
            "code_graph",
        ]),
        _ => with(&[
            "component_usage",
            "check_flows",
            "get_styles",
            "get_variables",
            "get_components",
            "create_frame",
            "create_text",
            "create_rectangle",
            "create_instance",
            "create_from_svg",
            "set_auto_layout",
            "set_sizing",
            "set_text",
            "set_text_props",
            "set_fill",
            "set_props",
            "set_image_fill",
            "move_to_parent",
            "delete_node",
            "select_nodes",
            "create_section",
            "project_memory",
            "tasks",
            "report",
            "get_design_context",
            "get_tokens",
            "export_assets",
            "motion_read",
            "motion_animate",
            "motion_to_lottie",
            "motion_check",
            "motion_tokens",
            "motion_to_code",
            // Listed here too: sessions on this profile fix code as well, and an unlisted tool loses to Grep.
            "code_graph",
        ]),
    }
}

/// Design sessions list design skills, code sessions code skills (area front matter); the rest stay loadable.
pub fn profile_area(profile: &str) -> Option<&'static str> {
    match profile {
        "core" | "prototype" => Some("design"),
        "code" => Some("code"),
        _ => None,
    }
}

pub fn tool_schema(t: &Tool) -> Value {
    zod::to_json_schema(&t.shape, DRAFT_2020_12)
}

// ---------- meta tools (index.mjs) ----------

fn use_file_tool() -> Tool {
    Tool {
        name: "use_file".into(),
        description: "Switch which open file wridge talks to when the plugin is open in more than one (e.g. a Figma design file and a FigJam board). Pass the file name, or \"figma\" / \"figjam\". get_document_info lists the open files. Check the file (get_document_info) again before changing it.".into(),
        shape: shape().f("file", string().min(1.0).describe("File name, or \"figma\" / \"figjam\"")).build(),
        run: handler(|ctx: Ctx, args: Value| async move { Ok(text_result(stringify(&ctx.bridge.use_file(args.get("file"))?))) }),
    }
}

/// "$hero" → the hero step's id; "$hero.width" → a property; "$0.id" → by step index.
fn resolve_refs(value: &Value, results: &HashMap<String, Value>) -> anyhow::Result<Value> {
    match value {
        Value::String(s) => {
            let re = regress::Regex::new(r"^\$(\w+)((?:\.\w+)*)$").unwrap();
            let Some(m) = re.find(s) else { return Ok(value.clone()) };
            let name = &s[m.group(1).unwrap()];
            let props = m.group(2).map(|r| &s[r]).unwrap_or("");
            let Some(out) = results.get(name) else {
                return Err(anyhow!("Unknown reference {s} — no earlier step named \"{name}\""));
            };
            if props.is_empty() {
                if let Value::Object(o) = out {
                    if let Some(id) = o.get("id") {
                        return Ok(id.clone());
                    }
                }
                return Ok(out.clone());
            }
            let mut cur: Option<Value> = Some(out.clone());
            for key in props[1..].split('.') {
                cur = match cur {
                    Some(Value::Object(o)) => o.get(key).cloned(),
                    Some(Value::Array(a)) => {
                        if key == "length" {
                            Some(json!(a.len()))
                        } else {
                            key.parse::<usize>().ok().filter(|_| !key.starts_with('0') || key == "0").and_then(|i| a.get(i).cloned())
                        }
                    }
                    Some(Value::String(st)) if key == "length" => Some(json!(crate::jsjson::len_utf16(&st))),
                    _ => None,
                };
            }
            cur.ok_or_else(|| anyhow!("Reference {s} resolved to nothing"))
        }
        Value::Array(a) => Ok(Value::Array(a.iter().map(|v| resolve_refs(v, results)).collect::<anyhow::Result<_>>()?)),
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, v) in o {
                out.insert(k.clone(), resolve_refs(v, results)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

fn result_value(result: &Value) -> Value {
    let Some(first) = result.get("content").and_then(|c| c.get(0)) else {
        return Value::Null;
    };
    if first.get("type") == Some(&json!("image")) {
        return json!("[image]");
    }
    match first.get("text") {
        Some(Value::String(t)) => serde_json::from_str::<Value>(t).unwrap_or_else(|_| Value::String(t.clone())),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn batch_tool() -> Tool {
    let step = object(
        shape()
            .f("tool", string())
            .f("args", record(unknown()).default(json!({})))
            .f("as", string().regex("^\\w+$", "").optional())
            .build(),
    );
    Tool {
        name: "run_batch".into(),
        description: "Run many tools in order in ONE call. Name a step with \"as\" and reference its result later: \"$hero\" (its id), \"$hero.width\", or by index \"$0\". Use this to build whole sections instead of writing scripts.".into(),
        shape: shape().f("steps", array(step).min(1.0).max(500.0)).f("stopOnError", boolean().default(json!(true))).build(),
        // the whole batch is one turn in the file's queue: no other session's command lands in between
        run: handler(|ctx: Ctx, args: Value| crate::session::hold(async move {
            let steps = args.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
            let stop_on_error = args.get("stopOnError") != Some(&json!(false));
            let mut results: HashMap<String, Value> = HashMap::new();
            let mut summary: Vec<Value> = vec![];
            let mut failed = 0usize;
            for (i, step) in steps.iter().enumerate() {
                let tool_name = step.get("tool").cloned().unwrap_or(Value::Null);
                let as_name = step.get("as").and_then(Value::as_str).map(str::to_string);
                let outcome: anyhow::Result<Value> = async {
                    let t = ctx.find_tool(Some(&tool_name)).filter(|t| !ctx.is_meta(&t.name));
                    let Some(t) = t else {
                        return Err(anyhow!("Unknown tool \"{}\" — see describe_tool or \"wara list\"", js_string(Some(&tool_name))));
                    };
                    let resolved = resolve_refs(step.get("args").unwrap_or(&json!({})), &results)?;
                    let parsed = zod::safe_parse(&t.shape, Some(&resolved)).map_err(|issues| anyhow!(prettify_error(&issues)))?;
                    let result = (t.run)(ctx.clone(), parsed).await?;
                    let value = result_value(&result);
                    if truthy(result.get("isError")) {
                        return Err(anyhow!(js_string(Some(&value))));
                    }
                    Ok(value)
                }
                .await;
                let mut entry = Map::new();
                entry.insert("step".into(), i.into());
                entry.insert("tool".into(), tool_name.clone());
                if let Some(a) = &as_name {
                    entry.insert("as".into(), a.clone().into());
                }
                match outcome {
                    Ok(value) => {
                        results.insert(i.to_string(), value.clone());
                        if let Some(a) = &as_name {
                            results.insert(a.clone(), value.clone());
                        }
                        if let Some(id) = value.as_object().and_then(|o| o.get("id")) {
                            entry.insert("id".into(), id.clone());
                        }
                        summary.push(Value::Object(entry));
                    }
                    Err(err) => {
                        failed += 1;
                        entry.insert("error".into(), err.to_string().into());
                        summary.push(Value::Object(entry));
                        if stop_on_error {
                            break;
                        }
                    }
                }
            }
            let mut refs = Map::new();
            for s in &steps {
                let Some(a) = s.get("as").and_then(Value::as_str) else { continue };
                if a.is_empty() || !truthy(results.get(a)) {
                    continue;
                }
                match results.get(a).and_then(|v| v.as_object()).and_then(|o| o.get("id")) {
                    Some(id) => {
                        refs.insert(a.to_string(), id.clone());
                    }
                    None => {
                        // the key holds undefined, which JSON leaves out (a later step with the same name still replaces it)
                        refs.shift_remove(a);
                    }
                }
            }
            let payload = json!({"completed": summary.len() - failed, "failed": failed, "total": steps.len(), "refs": refs, "steps": summary});
            Ok(json!({"content": [{"type": "text", "text": stringify(&payload)}], "isError": failed > 0 && stop_on_error}))
        })),
    }
}

fn call_tool() -> Tool {
    Tool {
        name: "call_tool".into(),
        description: "Call any wridge tool by name, including ones not listed in this session (see the instructions for their names). Use describe_tool first if unsure of its parameters.".into(),
        shape: shape().f("name", string()).f("args", record(unknown()).default(json!({}))).build(),
        run: handler(|ctx: Ctx, args: Value| async move {
            let name = args.get("name").cloned().unwrap_or(Value::Null);
            let t = ctx.find_tool(Some(&name)).filter(|t| !ctx.is_meta(&t.name));
            let Some(t) = t else {
                return Err(anyhow!("Unknown tool \"{}\" — see describe_tool", js_string(Some(&name))));
            };
            let parsed = zod::safe_parse(&t.shape, args.get("args")).map_err(|issues| anyhow!("{}\nExpected: {}", prettify_error(&issues), stringify(&tool_schema(&t))))?;
            (t.run)(ctx.clone(), parsed).await
        }),
    }
}

fn describe_tool() -> Tool {
    Tool {
        name: "describe_tool".into(),
        description: "Show a tool's description and parameters (JSON schema). Without a name, list every tool.".into(),
        shape: shape().f("name", string().optional()).build(),
        run: handler(|ctx: Ctx, args: Value| async move {
            let name = args.get("name");
            if !truthy(name) {
                let list = ctx
                    .tools
                    .iter()
                    .filter(|t| !ctx.is_meta(&t.name))
                    .map(|t| format!("{}: {}", t.name, t.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(text_result(list));
            }
            let Some(t) = ctx.find_tool(name) else {
                return Err(anyhow!("Unknown tool \"{}\"", js_string(name)));
            };
            Ok(text_result(stringify(
                &json!({"name": t.name, "description": t.description, "parameters": tool_schema(&t)}),
            )))
        }),
    }
}

fn get_skill_tool() -> Tool {
    Tool {
        name: "get_skill".into(),
        description: "Load a skill (design/coding method) by name before doing the matching work. Without a name, list skills.".into(),
        shape: shape().f("name", string().optional()).build(),
        run: handler(|ctx: Ctx, args: Value| async move {
            let skills = ctx.load_skills();
            let name = args.get("name");
            if !truthy(name) {
                return Ok(text_result(skill_index(&skills)));
            }
            let name = js_string(name);
            match skills.iter().find(|s| s.name == name) {
                Some(s) => Ok(text_result(s.body.clone())),
                None => Err(anyhow!(
                    "Unknown skill \"{name}\". Available: {}",
                    skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
                )),
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_resolve_like_index_mjs() {
        let mut results = HashMap::new();
        results.insert("hero".to_string(), json!({"id": "1:2", "width": 300, "kids": ["a", "b"]}));
        results.insert("0".to_string(), json!({"id": "9:9"}));
        results.insert("txt".to_string(), json!("abc"));
        let r = |v: Value| resolve_refs(&v, &results);
        assert_eq!(r(json!("$hero")).unwrap(), json!("1:2"));
        assert_eq!(r(json!("$hero.width")).unwrap(), json!(300));
        assert_eq!(r(json!("$hero.kids.1")).unwrap(), json!("b"));
        assert_eq!(r(json!("$hero.kids.length")).unwrap(), json!(2));
        assert_eq!(r(json!("$0.id")).unwrap(), json!("9:9"));
        assert_eq!(r(json!("$txt")).unwrap(), json!("abc"));
        assert_eq!(r(json!({"a": ["$hero", "plain"]})).unwrap(), json!({"a": ["1:2", "plain"]}));
        assert_eq!(r(json!("$nope")).unwrap_err().to_string(), "Unknown reference $nope — no earlier step named \"nope\"");
        assert_eq!(r(json!("$hero.missing")).unwrap_err().to_string(), "Reference $hero.missing resolved to nothing");
    }
}
