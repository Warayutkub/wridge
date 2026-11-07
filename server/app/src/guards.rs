//! Port of guards.mjs. Server-side guard: Figma writes only go to the file the AI checked
//! (get_document_info / design_map), and only while that file is still the one open. Plugin-side guards
//! (delete, approval, review) live in code.ts. Each session (session.rs) has its own check: one AI checking
//! a file never lets another AI write without checking.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::session::{self, SessionId};

use anyhow::anyhow;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::{json, Value};

/// Something that sends a command to the Figma plugin and waits for its answer.
pub trait PluginSend: Send + Sync {
    fn send(&self, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>>;
    /// Changes whenever a window closes or any session switches file: the guard's cached file name is
    /// only trusted while this stays the same.
    fn epoch(&self) -> u64 {
        0
    }
}

/// Tools that change the Figma document. Everything else reads, or works outside Figma.
pub const FIGMA_WRITES: &[&str] = &[
    "create_frame",
    "create_rectangle",
    "create_ellipse",
    "create_text",
    "create_instance",
    "create_component",
    "combine_as_variants",
    "create_section",
    "add_component_property",
    "create_paint_style",
    "create_text_style",
    "duplicate_node",
    "group_nodes",
    "ungroup_node",
    "set_text",
    "set_text_props",
    "set_fill",
    "set_stroke",
    "set_corner_radius",
    "set_effects",
    "set_props",
    "set_auto_layout",
    "set_sizing",
    "move_resize",
    "move_to_parent",
    "rename_node",
    "delete_node",
    "set_image_fill",
    "copy_fills",
    "create_polygon",
    "create_star",
    "create_line",
    "create_from_svg",
    "boolean_shapes",
    "flatten",
    "outline_stroke",
    "use_as_mask",
    "remove_mask",
    "add_interaction",
    "remove_interactions",
    "set_flow_start",
    "set_scroll",
    "apply_style",
    "bind_variable",
    "create_variable_collection",
    "create_variables",
    "set_variable_value",
    "set_instance_properties",
    "create_sticky",
    "create_shape",
    "create_connector",
    "figjam_draw",
    "motion_animate",
];

pub fn is_figma_write(command: &str) -> bool {
    FIGMA_WRITES.contains(&command)
}

type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Clone)]
struct GuardState {
    checked: Option<String>,
    /// The open file as last read: None = never read; Some(None) = the plugin said null/undefined.
    seen_name: Option<Value>,
    seen_at: f64,
    seen_epoch: u64,
}

/// Wraps send(command, args): remembers the file a check returned, and refuses writes when no file was
/// checked yet or another file is open now. The open file is re-read at most every `ttl_ms` (batches).
pub struct FileGuard {
    inner: Arc<dyn PluginSend>,
    ttl_ms: f64,
    now: Clock,
    state: Arc<Mutex<HashMap<SessionId, GuardState>>>,
}

impl GuardState {
    fn new() -> GuardState {
        GuardState {
            checked: None,
            seen_name: None,
            seen_at: f64::NEG_INFINITY,
            seen_epoch: 0,
        }
    }
}

/// `f` on the calling session's guard state.
fn with<R>(state: &Mutex<HashMap<SessionId, GuardState>>, key: &SessionId, f: impl FnOnce(&mut GuardState) -> R) -> R {
    let mut map = state.lock().unwrap();
    f(map.entry(key.clone()).or_insert_with(GuardState::new))
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// `${value}` in a JS template literal, for the file names in the guard's messages.
fn template(v: &Value) -> String {
    crate::jsjson::js_string(Some(v))
}

impl FileGuard {
    pub fn new(inner: Arc<dyn PluginSend>) -> FileGuard {
        FileGuard::with_clock(inner, 2000.0, Arc::new(now_ms))
    }

    pub fn with_clock(inner: Arc<dyn PluginSend>, ttl_ms: f64, now: Clock) -> FileGuard {
        FileGuard {
            inner,
            ttl_ms,
            now,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PluginSend for FileGuard {
    fn send(&self, command: &str, params: Value, timeout_ms: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
        let inner = self.inner.clone();
        let state = self.state.clone();
        let now = self.now.clone();
        let ttl = self.ttl_ms;
        let command = command.to_string();
        let key = session::current();
        async move {
            if is_figma_write(&command) {
                let checked = with(&state, &key, |g| g.checked.clone());
                let Some(checked) = checked else {
                    return Err(anyhow!("Check which Figma file is open before changing it: call get_document_info, and tell the user which file you will change."));
                };
                // openFile(): the cached name while fresh, else ask the plugin
                let epoch = inner.epoch();
                let cached = with(&state, &key, |s| if now() - s.seen_at < ttl && s.seen_epoch == epoch { s.seen_name.clone() } else { None });
                let open = match cached {
                    Some(name) => name,
                    None => {
                        let r = inner.send("file_name", json!({}), None).await?;
                        let name = match &r {
                            Value::Object(o) => o.get("fileName").cloned().unwrap_or(Value::String("\u{0}undefined".into())),
                            Value::Null => return Err(anyhow!("Cannot destructure property 'fileName' of '(intermediate value)' as it is null.")),
                            _ => Value::String("\u{0}undefined".into()),
                        };
                        with(&state, &key, |s| {
                            s.seen_name = Some(name.clone());
                            s.seen_at = now();
                            s.seen_epoch = epoch;
                        });
                        name
                    }
                };
                if open != Value::String(checked.clone()) {
                    with(&state, &key, |g| g.checked = None); // a new check is required, not just a retry
                    let shown = match &open {
                        Value::String(s) if s == "\u{0}undefined" => "undefined".to_string(),
                        v => template(v),
                    };
                    return Err(anyhow!(
                        "The open Figma file changed from \"{checked}\" to \"{shown}\" since you checked. Confirm with the user which file to change, then call get_document_info again."
                    ));
                }
            }
            let epoch_before = inner.epoch();
            let result = inner.send(&command, params, timeout_ms).await?;
            let mut map = state.lock().unwrap();
            let s = map.entry(key.clone()).or_insert_with(GuardState::new);
            let truthy_str = |v: Option<&Value>| match v {
                Some(Value::String(x)) if !x.is_empty() => Some(x.clone()),
                // a non-string truthy name is compared as-is by JS; names are strings in practice
                Some(v) if crate::jsjson::truthy(Some(v)) => Some(template(v)),
                _ => None,
            };
            if command == "get_document_info" {
                if let Some(name) = truthy_str(result.get("fileName")) {
                    s.checked = Some(name);
                }
            }
            if command == "design_map" {
                if let Some(name) = truthy_str(result.get("file")) {
                    s.checked = Some(name);
                }
            }
            if let Some(c) = s.checked.clone() {
                s.seen_name = Some(Value::String(c));
                s.seen_at = now();
                s.seen_epoch = epoch_before;
            }
            Ok(result)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    // Ported from legacy-node/test/guards.test.mjs
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct FakeFigma {
        file_name: Mutex<String>,
        calls: Mutex<Vec<String>>,
    }

    struct Fake(Arc<FakeFigma>);

    impl PluginSend for Fake {
        fn send(&self, command: &str, args: Value, _t: Option<u64>) -> BoxFuture<'static, anyhow::Result<Value>> {
            let f = self.0.clone();
            let command = command.to_string();
            async move {
                f.calls.lock().unwrap().push(command.clone());
                let name = f.file_name.lock().unwrap().clone();
                Ok(match command.as_str() {
                    "get_document_info" => json!({"fileName": name, "pages": []}),
                    "design_map" => json!({"file": name, "pages": []}),
                    "file_name" => json!({"fileName": name}),
                    _ => json!({"ok": true, "command": command, "args": args}),
                })
            }
            .boxed()
        }
    }

    fn fake(name: &str) -> Arc<FakeFigma> {
        Arc::new(FakeFigma {
            file_name: Mutex::new(name.into()),
            calls: Mutex::new(vec![]),
        })
    }

    fn clock(t: Arc<AtomicU64>) -> Clock {
        Arc::new(move || t.load(Ordering::SeqCst) as f64)
    }

    #[tokio::test]
    async fn a_write_before_any_file_check_is_refused_reads_pass() {
        let f = fake("utils");
        let send = FileGuard::new(Arc::new(Fake(f.clone())));
        let err = send.send("create_frame", json!({}), None).await.unwrap_err();
        assert!(err.to_string().contains("call get_document_info"));
        let r = send.send("get_node", json!({"nodeId": "1:2"}), None).await.unwrap();
        assert_eq!(r, json!({"ok": true, "command": "get_node", "args": {"nodeId": "1:2"}}));
    }

    #[tokio::test]
    async fn after_get_document_info_or_design_map_writes_go_through() {
        for check in ["get_document_info", "design_map"] {
            let f = fake("utils");
            let send = FileGuard::new(Arc::new(Fake(f.clone())));
            send.send(check, json!({}), None).await.unwrap();
            assert_eq!(send.send("create_frame", json!({"name": "A"}), None).await.unwrap()["ok"], json!(true));
        }
    }

    #[tokio::test]
    async fn another_file_opened_since_the_check_refused_and_a_new_check_is_required() {
        let t = Arc::new(AtomicU64::new(0));
        let f = fake("utils");
        let send = FileGuard::with_clock(Arc::new(Fake(f.clone())), 2000.0, clock(t.clone()));
        send.send("get_document_info", json!({}), None).await.unwrap();
        *f.file_name.lock().unwrap() = "flowklass".into();
        t.store(5000, Ordering::SeqCst);
        let err = send.send("rename_node", json!({}), None).await.unwrap_err();
        assert!(err.to_string().contains("changed from \"utils\" to \"flowklass\""), "{err}");
        t.store(5001, Ordering::SeqCst);
        let err = send.send("rename_node", json!({}), None).await.unwrap_err();
        assert!(err.to_string().contains("call get_document_info"));
        send.send("get_document_info", json!({}), None).await.unwrap();
        assert_eq!(send.send("rename_node", json!({}), None).await.unwrap()["ok"], json!(true));
    }

    #[tokio::test]
    async fn the_open_file_is_re_read_at_most_once_per_ttl() {
        let t = Arc::new(AtomicU64::new(0));
        let f = fake("utils");
        let send = FileGuard::with_clock(Arc::new(Fake(f.clone())), 2000.0, clock(t.clone()));
        send.send("get_document_info", json!({}), None).await.unwrap();
        t.store(3000, Ordering::SeqCst);
        for _ in 0..5 {
            send.send("create_text", json!({}), None).await.unwrap();
        }
        assert_eq!(f.calls.lock().unwrap().iter().filter(|c| *c == "file_name").count(), 1);
    }

    #[test]
    fn every_figma_write_tool_is_guarded_code_data_tools_are_not() {
        for name in [
            "create_frame",
            "delete_node",
            "rename_node",
            "set_props",
            "create_section",
            "add_interaction",
            "set_flow_start",
        ] {
            assert!(is_figma_write(name), "{name}");
        }
        for name in ["get_design_context", "code_graph", "project_memory", "map_component", "export_assets", "select_nodes"] {
            assert!(!is_figma_write(name), "{name}");
        }
    }
}
