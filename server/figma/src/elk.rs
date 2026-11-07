//! elkjs (the unchanged `elk.bundled.js` from legacy-node/node_modules) running inside QuickJS.
//!
//! One QuickJS runtime lives on its own thread for the life of the process: loading the 1.6 MB
//! bundle costs far more than a layout, so it is loaded once and every layout after that reuses
//! it (as Node keeps its `new ELK()` instance). Calls from any thread are queued to that thread.

use anyhow::{anyhow, Result};
use rquickjs::{CatchResultExt, Context, Function, Object, Runtime};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

const ELK_JS: &str = include_str!("../assets/elk.bundled.js");

// What the bundle expects from a browser / Node: `global` (it reads Math, setTimeout… from it),
// `setTimeout` (its fake worker answers asynchronously) and a CommonJS `module`.
const PRELUDE: &str = r#"
globalThis.global = globalThis;
var __timers = [];
globalThis.setTimeout = function (fn) { __timers.push(fn); return __timers.length; };
globalThis.clearTimeout = function () {};
globalThis.console = { log: function () {}, warn: function () {}, error: function () {}, err: undefined };
var module = { exports: {} };
var exports = module.exports;
"#;

const SETUP: &str = r#"
var __elk = new module.exports();
globalThis.__layout = function (json) {
  var state = { done: false, nums: null, error: null };
  __elk.layout(JSON.parse(json)).then(function (g) {
    var out = [g.width, g.height];
    for (var i = 0; i < g.children.length; i++) out.push(g.children[i].x, g.children[i].y);
    state.nums = out;
    state.done = true;
  }, function (e) {
    state.error = String(e && e.message !== undefined ? e.message : e);
    state.done = true;
  });
  return state;
};
// Node runs one timer, then the microtasks it queued, then the next timer.
globalThis.__tick = function () {
  if (!__timers.length) return false;
  var fn = __timers.shift();
  fn();
  return true;
};
"#;

/// Result of one ELK layout: the graph size and each child's top-left corner, in input order.
#[derive(Debug, Clone, PartialEq)]
pub struct ElkResult {
    pub width: f64,
    pub height: f64,
    pub at: Vec<(f64, f64)>,
}

struct Engine {
    _rt: Runtime,
    ctx: Context,
}

impl Engine {
    fn new() -> Result<Self> {
        let rt = Runtime::new()?;
        rt.set_max_stack_size(64 * 1024 * 1024);
        let ctx = Context::full(&rt)?;
        ctx.with(|ctx| -> Result<()> {
            ctx.eval::<(), _>(PRELUDE).catch(&ctx).map_err(|e| anyhow!("ELK prelude: {e}"))?;
            ctx.eval::<(), _>(ELK_JS).catch(&ctx).map_err(|e| anyhow!("Loading elk.bundled.js: {e}"))?;
            ctx.eval::<(), _>(SETUP).catch(&ctx).map_err(|e| anyhow!("ELK setup: {e}"))?;
            Ok(())
        })?;
        Ok(Engine { _rt: rt, ctx })
    }

    fn layout(&self, graph_json: &str) -> Result<ElkResult> {
        self.ctx.with(|ctx| -> Result<ElkResult> {
            let g = ctx.globals();
            let layout: Function = g.get("__layout")?;
            let tick: Function = g.get("__tick")?;
            let state: Object = layout.call((graph_json,)).catch(&ctx).map_err(|e| anyhow!("{e}"))?;
            loop {
                while ctx.execute_pending_job() {}
                if state.get::<_, bool>("done")? {
                    break;
                }
                let ran: bool = tick.call(()).catch(&ctx).map_err(|e| anyhow!("{e}"))?;
                if !ran && !ctx.execute_pending_job() && !state.get::<_, bool>("done")? {
                    return Err(anyhow!("ELK layout did not finish"));
                }
            }
            if let Some(err) = state.get::<_, Option<String>>("error")? {
                return Err(anyhow!(err));
            }
            let nums: Vec<f64> = state.get("nums")?;
            let at = nums[2..].chunks(2).map(|c| (c[0], c[1])).collect();
            Ok(ElkResult { width: nums[0], height: nums[1], at })
        })
    }
}

type Job = (String, Sender<Result<ElkResult>>);

fn spawn() -> Result<Sender<Job>> {
    let (tx, rx) = channel::<Job>();
    std::thread::Builder::new()
        .name("wridge-elk".into())
        .stack_size(128 * 1024 * 1024)
        .spawn(move || {
            let engine = Engine::new();
            for (json, reply) in rx {
                let res = match &engine {
                    Ok(e) => e.layout(&json),
                    Err(e) => Err(anyhow!("{e}")),
                };
                let _ = reply.send(res);
            }
        })
        .map_err(|e| anyhow!("Cannot start the ELK thread: {e}"))?;
    Ok(tx)
}

// The ELK thread's queue; replaced by a fresh thread if the old one ever stops.
fn queue() -> &'static Mutex<Option<Sender<Job>>> {
    static Q: OnceLock<Mutex<Option<Sender<Job>>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(None))
}

/// Lay out an ELK JSON graph (as text). Blocks until the ELK thread answers; the first call also
/// loads elkjs (see [`warm_up`]).
pub fn layout(graph_json: &str) -> Result<ElkResult> {
    let (tx, rx) = channel();
    {
        let mut q = queue().lock().unwrap_or_else(|p| p.into_inner());
        let job = (graph_json.to_string(), tx);
        // not sent yet: no thread, or the old one is gone
        let unsent = match q.as_ref() {
            Some(s) => s.send(job).err().map(|e| e.0),
            None => Some(job),
        };
        if let Some(job) = unsent {
            let s = spawn()?;
            s.send(job).map_err(|_| anyhow!("ELK thread stopped"))?;
            *q = Some(s);
        }
    }
    rx.recv().map_err(|_| {
        // the thread died on this graph: the next call starts a new one
        *queue().lock().unwrap_or_else(|p| p.into_inner()) = None;
        anyhow!("ELK stopped while laying out this diagram")
    })?
}

/// Load elkjs now (e.g. at server start) so the first diagram doesn't pay for it.
pub fn warm_up() -> Result<()> {
    layout(r#"{"id":"root","children":[],"edges":[]}"#).map(|_| ())
}
