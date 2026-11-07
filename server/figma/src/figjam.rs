//! Port of `legacy-node/figjam.mjs`: lays out a system-design diagram for FigJam. The AI sends boxes and
//! arrows (with lanes for who does each step), tables, or participants and messages; this file decides
//! where everything goes. Placement uses ELK (layered), run unchanged inside QuickJS (see [`crate::elk`]).

use crate::elk;
use crate::js::{self, num, Obj};
use crate::obj;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

struct Kind {
    shape: Option<&'static str>,
    w: f64,
    h: f64,
    fill: Option<&'static str>,
    stroke: Option<&'static str>,
    text: Option<&'static str>,
    prefix: Option<&'static str>,
    sticky: Option<&'static str>,
}

const fn kind(shape: Option<&'static str>) -> Kind {
    Kind { shape, w: 0.0, h: 0.0, fill: None, stroke: None, text: None, prefix: None, sticky: None }
}
const fn sticky(c: &'static str) -> Kind {
    Kind { shape: None, w: 0.0, h: 0.0, fill: None, stroke: None, text: None, prefix: None, sticky: Some(c) }
}

// Look of each kind of node. Colours follow the board conventions in the system-design skill.
fn kind_of(k: &str) -> Option<Kind> {
    Some(match k {
        "start" => Kind { w: 56.0, h: 56.0, fill: Some("#0B0D12"), text: Some(""), ..kind(Some("ELLIPSE")) },
        "end" => Kind { w: 56.0, h: 56.0, fill: Some("#FFFFFF"), stroke: Some("#0B0D12"), text: Some(""), ..kind(Some("ELLIPSE")) },
        "step" => kind(Some("ROUNDED_RECTANGLE")),
        "wait" => Kind { prefix: Some("⏳ "), ..kind(Some("ROUNDED_RECTANGLE")) },
        "decision" => kind(Some("DIAMOND")),
        "state" => kind(Some("ELLIPSE")),
        "table" => kind(Some("SQUARE")),
        "question" => sticky("#FDE68A"),
        "risk" => sticky("#FECACA"),
        "agreed" => sticky("#BBF7D0"),
        "note" => sticky("#E2E8F0"),
        _ => return None,
    })
}

/// `NODE_KINDS`.
pub const NODE_KINDS: [&str; 11] = ["start", "end", "step", "wait", "decision", "state", "table", "question", "risk", "agreed", "note"];
/// `NODE_STATUS`: existing = already in the product (grey), new = proposed (blue), changed (amber), removed (red outline).
pub const NODE_STATUS: [&str; 4] = ["existing", "new", "changed", "removed"];
/// `DIAGRAM_TYPES`.
pub const DIAGRAM_TYPES: [&str; 4] = ["flow", "state", "er", "sequence"];

struct Status {
    fill: &'static str,
    stroke: &'static str,
    text_color: &'static str,
}
fn status_of(s: Option<&Value>) -> Status {
    let key = js::nullish(s).map_or("new".to_string(), js::string);
    match key.as_str() {
        "existing" => Status { fill: "#F1F5F9", stroke: "#CBD5E1", text_color: "#334155" },
        "changed" => Status { fill: "#FEF3C7", stroke: "#D97706", text_color: "#0B0D12" },
        "removed" => Status { fill: "#FFFFFF", stroke: "#DC2626", text_color: "#991B1B" },
        _ => Status { fill: "#DBEAFE", stroke: "#2563EB", text_color: "#0B0D12" },
    }
}

// TITLE leaves room for FigJam's own section label, which sits inside the section's top-left corner.
const LANE_LABEL: f64 = 200.0;
const PAD: f64 = 64.0;
const LANE_PAD: f64 = 36.0;
const TITLE: f64 = 140.0;
const NOTE: f64 = 280.0;

// Thai marks above/below a letter take no width, so they are not counted.
fn visible(t: Option<&Value>) -> String {
    let s = js::nullish(t).map(js::string).unwrap_or_default();
    s.chars().filter(|c| !matches!(*c, '\u{e31}' | '\u{e34}'..='\u{e3a}' | '\u{e47}'..='\u{e4e}')).collect()
}
fn vlen(t: Option<&Value>) -> f64 {
    js::utf16_len(&visible(t)) as f64
}

// Box size from its text: FigJam wraps inside the shape, so make the shape fit a few short lines.
fn fit(text: &str, fs: f64, min_w: f64, max_w: f64, min_h: f64) -> (f64, f64) {
    let per = fs * 0.62; // average glyph width at this size
    let chars = vlen(Some(&Value::String(text.into())));
    let w = js::max(min_w, js::min(max_w, chars * per + 48.0));
    let lines = js::max(1.0, ((chars * per) / (w - 48.0)).ceil());
    (js::round(w), js::round(js::max(min_h, lines * fs * 1.45 + 40.0)))
}

// Edges that close a loop (retry, go back) — drawn dashed.
fn back_edges(ids: &[String], edges: &[(String, String)]) -> HashSet<String> {
    let mut out: HashMap<&str, Vec<&str>> = ids.iter().map(|i| (i.as_str(), vec![])).collect();
    for (f, t) in edges {
        if out.contains_key(f.as_str()) && out.contains_key(t.as_str()) {
            out.get_mut(f.as_str()).unwrap().push(t.as_str());
        }
    }
    let mut state: HashMap<&str, u8> = HashMap::new();
    let mut back = HashSet::new();
    fn visit<'a>(id: &'a str, out: &HashMap<&'a str, Vec<&'a str>>, state: &mut HashMap<&'a str, u8>, back: &mut HashSet<String>) {
        state.insert(id, 1);
        for &to in &out[id] {
            match state.get(to) {
                Some(1) => {
                    back.insert(format!("{id}>{to}"));
                }
                None => visit(to, out, state, back),
                _ => {}
            }
        }
        state.insert(id, 2);
    }
    let has_in: HashSet<&str> = edges.iter().map(|(_, t)| t.as_str()).collect();
    for id in ids {
        if !has_in.contains(id.as_str()) && !state.contains_key(id.as_str()) {
            visit(id, &out, &mut state, &mut back);
        }
    }
    for id in ids {
        if !state.contains_key(id.as_str()) {
            visit(id, &out, &mut state, &mut back);
        }
    }
    back
}

struct Placed {
    at: HashMap<String, (f64, f64)>,
    width: f64,
    height: f64,
}

// ELK layered layout, left → right: fewer crossings, and room between columns for the arrow labels.
fn elk_place(boxes: &[(String, f64, f64)], edges: &[(String, String, Option<Value>)], node_gap: f64, layer_gap: f64) -> Result<Placed> {
    let graph = obj! {
        "id" => "root",
        "layoutOptions" => obj! {
            "elk.algorithm" => "layered",
            "elk.direction" => "RIGHT",
            "elk.spacing.nodeNode" => js::num_str(node_gap),
            "elk.layered.spacing.nodeNodeBetweenLayers" => js::num_str(layer_gap),
            "elk.layered.spacing.edgeNodeBetweenLayers" => "40",
            "elk.spacing.edgeLabel" => "10",
            "elk.layered.crossingMinimization.strategy" => "LAYER_SWEEP",
            "elk.layered.nodePlacement.strategy" => "NETWORK_SIMPLEX",
            "elk.layered.considerModelOrder.strategy" => "NODES_AND_EDGES",
            "elk.layered.cycleBreaking.strategy" => "DEPTH_FIRST",
        },
        "children" => Value::Array(boxes.iter().map(|(id, w, h)| obj! { "id" => id.as_str(), "width" => *w, "height" => *h }).collect()),
        "edges" => Value::Array(edges.iter().enumerate().map(|(i, (f, t, label))| {
            let labels = if js::truthy(label.as_ref()) {
                vec![obj! { "text" => label.clone(), "width" => vlen(label.as_ref()) * 8.0 + 20.0, "height" => 22i64 }]
            } else {
                vec![]
            };
            obj! { "id" => format!("elk{i}"), "sources" => vec![Value::String(f.clone())], "targets" => vec![Value::String(t.clone())], "labels" => labels }
        }).collect()),
    };
    let r = elk::layout(&js::stringify(&graph))?;
    let at = boxes.iter().zip(r.at).map(|((id, _, _), p)| (id.clone(), p)).collect();
    Ok(Placed { at, width: r.width, height: r.height })
}

// Stickies in up to two columns beside the diagram (shared by every type).
fn place_notes(notes: &[Value], x0: f64, top: f64) -> Result<Vec<Value>> {
    let mut out = vec![];
    for (i, nt) in notes.iter().enumerate() {
        let n = Some(nt);
        let kind = js::nullish(js::get(n, "kind")).map_or("note".to_string(), js::string);
        let Some(color) = kind_of(&kind).and_then(|k| k.sticky) else {
            return Err(anyhow!("Note kind \"{kind}\" — use question, risk, agreed or note"));
        };
        out.push(obj! {
            "id" => js::nullish(js::get(n, "id")).cloned().unwrap_or_else(|| Value::String(format!("note{}", i + 1))),
            "kind" => "sticky", "role" => kind.as_str(),
            "x" => x0 + (i % 2) as f64 * NOTE, "y" => top + (i / 2) as f64 * NOTE,
            "text" => js::get(n, "text").cloned(), "fill" => color,
        });
    }
    Ok(out)
}

fn notes_size(n: usize) -> (f64, f64) {
    if n == 0 {
        (0.0, 0.0)
    } else {
        ((n.min(2)) as f64 * NOTE, (n as f64 / 2.0).ceil() * NOTE)
    }
}

fn section(spec: &Value, fallback: &str, notes: usize, width: f64, height: f64) -> Value {
    let sp = Some(spec);
    let (nw, nh) = notes_size(notes);
    let mut m = Obj::new();
    m.insert("name".into(), js::nullish(js::get(sp, "title")).cloned().unwrap_or_else(|| fallback.into()));
    if let Some(x) = js::nullish(js::get(sp, "x")) {
        m.insert("x".into(), x.clone());
        m.insert("y".into(), js::nullish(js::get(sp, "y")).cloned().unwrap_or(Value::from(0)));
    }
    m.insert("width".into(), num(js::round(width + if nw != 0.0 { nw + 24.0 } else { 0.0 } + PAD)));
    m.insert("height".into(), num(js::round(js::max(height, TITLE + nh) + PAD)));
    m.insert("fill".into(), "#FFFFFF".into());
    Value::Object(m)
}

fn notes_of(spec: &Value) -> Result<Vec<Value>> {
    match js::nullish(js::get(Some(spec), "notes")) {
        None => Ok(vec![]),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(_) => Err(anyhow!("(spec.notes ?? []).map is not a function")),
    }
}

/// `layoutDiagram(spec)`: dispatch on spec.type — flow (default, lanes = actors), state (state machine),
/// er (tables), sequence. Blocks while ELK lays the diagram out.
pub fn layout_diagram(spec: &Value) -> Result<Value> {
    let t = js::nullish(js::get(Some(spec), "type")).map_or("flow".to_string(), js::string);
    match t.as_str() {
        "flow" | "state" => layout_flow(spec),
        "er" => layout_er(spec),
        "sequence" => layout_sequence(spec),
        _ => Err(anyhow!("Unknown diagram type \"{t}\" — use {}", DIAGRAM_TYPES.join(", "))),
    }
}

struct FlowNode {
    raw: Value,
    id: String,
    kind: String,
    text: String,
    w: f64,
    h: f64,
}

// flow / state: { lanes?, nodes: [{ id, text, kind?, lane?, status? }], edges: [{ from, to, label?, dashed? }] }
fn layout_flow(spec: &Value) -> Result<Value> {
    let sp = Some(spec);
    let is_state = js::get(sp, "type") == Some(&Value::String("state".into()));
    let mut ids: Vec<String> = vec![];
    let mut nodes: Vec<FlowNode> = vec![];
    let list = match js::nullish(js::get(sp, "nodes")) {
        None => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(anyhow!("(spec.nodes ?? []).map is not a function")),
    };
    for (i, n) in list.iter().enumerate() {
        let nv = Some(n);
        let id = js::nullish(js::get(nv, "id")).map_or(format!("n{}", i + 1), js::string);
        if ids.contains(&id) {
            return Err(anyhow!("Duplicate node id \"{id}\""));
        }
        ids.push(id.clone());
        let kind = js::nullish(js::get(nv, "kind")).map_or(if is_state { "state".to_string() } else { "step".to_string() }, js::string);
        let Some(k) = kind_of(&kind) else {
            return Err(anyhow!("Unknown kind \"{kind}\" for {id} — use {}", NODE_KINDS.join(", ")));
        };
        if k.sticky.is_some() {
            return Err(anyhow!("\"{kind}\" is a sticky: put it in notes, not nodes"));
        }
        let text = match k.text {
            Some(t) => t.to_string(),
            None => format!("{}{}", k.prefix.unwrap_or(""), js::nullish(js::get(nv, "text")).map(js::string).unwrap_or_default()),
        };
        let (w, h) = match kind.as_str() {
            "start" | "end" => (k.w, k.h),
            "decision" => fit(&text, 14.0, 190.0, 240.0, 130.0),
            "state" => fit(&text, 16.0, 190.0, 250.0, 84.0),
            _ => fit(&text, 16.0, 200.0, 260.0, 88.0),
        };
        nodes.push(FlowNode { raw: n.clone(), id, kind, text, w, h });
    }
    let edges_in = match js::nullish(js::get(sp, "edges")) {
        None => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(anyhow!("spec.edges is not iterable")),
    };
    let id_of = |v: Option<&Value>| -> Option<String> {
        match v {
            Some(Value::String(s)) if ids.contains(s) => Some(s.clone()),
            _ => None,
        }
    };
    let mut edges: Vec<(String, String)> = vec![];
    for e in &edges_in {
        let ev = Some(e);
        match (id_of(js::get(ev, "from")), id_of(js::get(ev, "to"))) {
            (Some(f), Some(t)) => edges.push((f, t)),
            _ => {
                return Err(anyhow!(
                    "Edge {} → {}: unknown node (ids: {})",
                    js::to_string(js::get(ev, "from")),
                    js::to_string(js::get(ev, "to")),
                    ids.join(", ")
                ))
            }
        }
    }

    let lane_names: Vec<Value> = match js::get(sp, "lanes") {
        Some(Value::Array(l)) if !l.is_empty() => l.clone(),
        _ => {
            let mut seen: Vec<Value> = vec![];
            for n in &nodes {
                if let Some(l) = js::get(Some(&n.raw), "lane").filter(|l| js::truthy(Some(l))) {
                    if !seen.iter().any(|s| js::strict_eq(Some(s), Some(l))) {
                        seen.push(l.clone());
                    }
                }
            }
            seen
        }
    };
    let has_lanes = !lane_names.is_empty();
    let index_of = |v: Option<&Value>| lane_names.iter().position(|l| js::strict_eq(Some(l), v));
    for n in &nodes {
        let lane = js::get(Some(&n.raw), "lane");
        if has_lanes && js::truthy(lane) && index_of(lane).is_none() {
            return Err(anyhow!("Node {}: lane \"{}\" is not in lanes ({})", n.id, js::to_string(lane), js::join(&lane_names, ", ")));
        }
    }
    let lane_of = |n: &FlowNode| -> usize {
        if !has_lanes {
            return 0;
        }
        let lane = js::nullish(js::get(Some(&n.raw), "lane")).or(lane_names.first());
        index_of(lane).unwrap_or(0)
    };

    let back = back_edges(&ids, &edges);
    let boxes: Vec<(String, f64, f64)> = nodes.iter().map(|n| (n.id.clone(), n.w, n.h)).collect();
    let elk_edges: Vec<(String, String, Option<Value>)> = edges.iter().zip(&edges_in).map(|((f, t), e)| (f.clone(), t.clone(), js::get(Some(e), "label").cloned())).collect();
    let placed = elk_place(&boxes, &elk_edges, if is_state { 72.0 } else { 56.0 }, if is_state { 150.0 } else { 110.0 })?;
    let at = |id: &str| placed.at[id];
    let left = if has_lanes { LANE_LABEL } else { 0.0 } + PAD;
    let width = left + placed.width;
    let mut pos: HashMap<String, (f64, f64)> = HashMap::new();
    let mut lanes: Vec<Value> = vec![];
    let diagram_h = if !has_lanes {
        for n in &nodes {
            let (x, y) = at(&n.id);
            pos.insert(n.id.clone(), (left + x, TITLE + y));
        }
        TITLE + placed.height
    } else {
        // Keep ELK's left → right order; each box goes into its actor's lane. Boxes of one lane that would
        // overlap side by side take another row of that lane (in ELK's top → bottom order).
        const GAP: f64 = 40.0;
        let mut rows_of: Vec<Vec<Vec<(f64, f64)>>> = lane_names.iter().map(|_| vec![]).collect();
        let mut slot: HashMap<String, usize> = HashMap::new();
        let mut order: Vec<&FlowNode> = nodes.iter().collect();
        order.sort_by(|a, b| js::cmp_num(at(&a.id).1 - at(&b.id).1));
        for n in order {
            let l = lane_of(n);
            let x0 = at(&n.id).0;
            let x1 = x0 + n.w;
            let r = match rows_of[l].iter().position(|row| row.iter().all(|&(a, b)| x1 + GAP <= a || x0 >= b + GAP)) {
                Some(r) => r,
                None => {
                    rows_of[l].push(vec![]);
                    rows_of[l].len() - 1
                }
            };
            rows_of[l][r].push((x0, x1));
            slot.insert(n.id.clone(), r);
        }
        let row_h = js::max_of(nodes.iter().map(|n| n.h)) + 40.0;
        let mut y = TITLE;
        let mut lane_y = vec![];
        for (l, name) in lane_names.iter().enumerate() {
            let height = js::max(1.0, rows_of[l].len() as f64) * row_h + LANE_PAD * 2.0;
            lanes.push(obj! { "name" => name, "x" => 0i64, "y" => y, "width" => width + PAD, "height" => height, "fill" => if l % 2 == 1 { "#FFFFFF" } else { "#F8FAFC" } });
            lane_y.push(y);
            y += height;
        }
        for n in &nodes {
            let ly = lane_y[lane_of(n)];
            pos.insert(n.id.clone(), (left + at(&n.id).0, ly + LANE_PAD + slot[&n.id] as f64 * row_h + (row_h - n.h) / 2.0));
        }
        y
    };
    let mut out_nodes: Vec<Value> = nodes
        .iter()
        .map(|n| {
            let k = kind_of(&n.kind).unwrap();
            let st = status_of(js::get(Some(&n.raw), "status"));
            let (x, y) = pos[&n.id];
            obj! {
                "id" => n.id.as_str(), "kind" => "shape", "shape" => k.shape.unwrap_or(""), "role" => n.kind.as_str(),
                "x" => js::round(x), "y" => js::round(y), "width" => n.w, "height" => n.h, "text" => n.text.as_str(),
                "fill" => k.fill.unwrap_or(st.fill), "stroke" => k.stroke.unwrap_or(st.stroke),
                "textColor" => if n.kind == "start" { "#FFFFFF" } else { st.text_color },
                "fontSize" => if n.kind == "decision" { 14i64 } else { 16 },
            }
        })
        .collect();
    let notes = notes_of(spec)?;
    out_nodes.extend(place_notes(&notes, width + PAD + 24.0, TITLE)?);
    let out_edges: Vec<Value> = edges_in
        .iter()
        .zip(&edges)
        .enumerate()
        .map(|(i, (e, (f, t)))| {
            let ev = Some(e);
            let is_back = back.contains(&format!("{f}>{t}"));
            let dashed = match js::get(ev, "dashed") {
                Some(d) if js::truthy(Some(d)) => Some(d.clone()),
                _ if is_back => Some(Value::Bool(true)),
                _ => None,
            };
            obj! {
                "id" => js::nullish(js::get(ev, "id")).cloned().unwrap_or_else(|| Value::String(format!("e{}", i + 1))),
                "from" => js::get(ev, "from").cloned(), "to" => js::get(ev, "to").cloned(), "label" => js::get(ev, "label").cloned(),
                "dashed" => dashed, "color" => if is_back { "#94A3B8" } else { "#334155" },
            }
        })
        .collect();
    Ok(obj! {
        "section" => section(spec, "System design · Draft", notes.len(), width + PAD, diagram_h),
        "lanes" => Value::Array(lanes), "nodes" => Value::Array(out_nodes), "edges" => Value::Array(out_edges),
    })
}

// ER: one box per table (name, then a line per column: 🔑 PK, → FK). ELK places them with the referenced
// tables to the left; FK arrows point from the referencing table to the referenced one.
fn layout_er(spec: &Value) -> Result<Value> {
    let sp = Some(spec);
    let tables = match js::nullish(js::get(sp, "tables")) {
        Some(Value::Array(t)) if !t.is_empty() => t.clone(),
        _ => return Err(anyhow!("er needs tables: [{{ id, name, columns: [{{ name, type, pk?, fk? }}] }}]")),
    };
    let key_of = |t: &Value| js::nullish(js::get(Some(t), "id")).or_else(|| js::get(Some(t), "name")).map(js::string).unwrap_or_else(|| "undefined".into());
    let names: Vec<String> = tables.iter().map(key_of).collect();
    let columns = |t: &Value| -> Vec<Value> { js::nullish(js::get(Some(t), "columns")).and_then(|c| c.as_array()).cloned().unwrap_or_default() };
    let mut edges: Vec<(String, String, Option<Value>)> = vec![];
    for t in &tables {
        for c in columns(t) {
            let cv = Some(&c);
            let fk = js::get(cv, "fk");
            if !js::truthy(fk) {
                continue;
            }
            let target = js::to_string(fk).split('.').next().unwrap_or("").to_string();
            if !names.contains(&target) {
                return Err(anyhow!(
                    "{}.{} → {}: table \"{target}\" is not in tables",
                    js::to_string(js::get(Some(t), "name")),
                    js::to_string(js::get(cv, "name")),
                    js::to_string(fk)
                ));
            }
            edges.push((key_of(t), target, js::get(cv, "name").cloned()));
        }
    }
    struct B {
        id: String,
        t: Value,
        lines: Vec<String>,
        w: f64,
        h: f64,
    }
    let boxes: Vec<B> = tables
        .iter()
        .map(|t| {
            let lines: Vec<String> = columns(t)
                .iter()
                .map(|c| {
                    let cv = Some(c);
                    let fk = js::get(cv, "fk");
                    format!(
                        "{}{}  {}{}{}",
                        if js::truthy(js::get(cv, "pk")) { "🔑 " } else if js::truthy(fk) { "→ " } else { "• " },
                        js::to_string(js::get(cv, "name")),
                        js::nullish(js::get(cv, "type")).map(js::string).unwrap_or_default(),
                        if js::truthy(fk) { format!("  ({})", js::to_string(fk)) } else { String::new() },
                        if js::truthy(js::get(cv, "note")) { format!(" · {}", js::to_string(js::get(cv, "note"))) } else { String::new() },
                    )
                })
                .collect();
            let longest = js::max_of(std::iter::once(vlen(js::get(Some(t), "name")) + 4.0).chain(lines.iter().map(|l| vlen(Some(&Value::String(l.clone()))))));
            let w = js::round(js::min(400.0, js::max(240.0, longest * 8.2 + 40.0)));
            let h = js::round(64.0 + js::max(1.0, lines.len() as f64) * 21.0 + 24.0);
            B { id: key_of(t), t: t.clone(), lines, w, h }
        })
        .collect();
    // laid out referenced → referencing, so referenced tables come first (left)
    let placed = elk_place(
        &boxes.iter().map(|b| (b.id.clone(), b.w, b.h)).collect::<Vec<_>>(),
        &edges.iter().map(|(f, t, l)| (t.clone(), f.clone(), l.clone())).collect::<Vec<_>>(),
        48.0,
        140.0,
    )?;
    let mut out: Vec<Value> = boxes
        .iter()
        .map(|b| {
            let st = status_of(js::get(Some(&b.t), "status"));
            let (x, y) = placed.at[&b.id];
            obj! {
                "id" => b.id.as_str(), "kind" => "shape", "shape" => "SQUARE", "role" => "table",
                "x" => js::round(PAD + x), "y" => js::round(TITLE + y), "width" => b.w, "height" => b.h,
                "text" => format!("{}\n\n{}", js::to_string(js::get(Some(&b.t), "name")), b.lines.join("\n")),
                "fill" => st.fill, "stroke" => st.stroke, "textColor" => st.text_color, "fontSize" => 13i64,
            }
        })
        .collect();
    let width = PAD + placed.width + PAD;
    let notes = notes_of(spec)?;
    out.extend(place_notes(&notes, width + 24.0, TITLE)?);
    Ok(obj! {
        "section" => section(spec, "Data · System design · Draft", notes.len(), width, TITLE + placed.height),
        "lanes" => Value::Array(vec![]), "nodes" => Value::Array(out),
        "edges" => Value::Array(edges.iter().enumerate().map(|(i, (f, t, l))| obj! { "id" => format!("fk{}", i + 1), "from" => f.as_str(), "to" => t.as_str(), "label" => l.clone(), "color" => "#2563EB", "lineType" => "ELBOWED" }).collect()),
    })
}

// Sequence: participants across the top, a dashed lifeline under each, then one row per message
// (arrows between lifelines, top to bottom in time order). reply = dashed.
fn layout_sequence(spec: &Value) -> Result<Value> {
    let sp = Some(spec);
    let parts = js::nullish(js::get(sp, "participants")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let msgs = js::nullish(js::get(sp, "messages")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    if parts.len() < 2 || msgs.is_empty() {
        return Err(anyhow!("sequence needs participants (2+) and messages: [{{ from, to, text, kind? }}]"));
    }
    // a Map from participant to its column: a repeated name takes its last column
    let col = |v: Option<&Value>| parts.iter().rposition(|p| js::strict_eq(Some(p), v));
    for m in &msgs {
        let mv = Some(m);
        let (f, t) = (js::get(mv, "from"), js::get(mv, "to"));
        if col(f).is_none() || col(t).is_none() {
            return Err(anyhow!(
                "Message \"{}\": {} is not a participant ({})",
                js::to_string(js::get(mv, "text")),
                js::to_string(if col(f).is_none() { f } else { t }),
                js::join(&parts, ", ")
            ));
        }
    }
    // columns wide enough for the longest message between neighbours
    let longest = js::max_of(msgs.iter().map(|m| vlen(js::get(Some(m), "text")) + 4.0));
    let col_w = js::max(300.0, js::min(460.0, longest * 8.0 + 60.0));
    let (head_w, head_h, row) = (200.0, 64.0, 80.0);
    let top = TITLE + head_h + 48.0;
    let cx = |i: usize| PAD + i as f64 * col_w + head_w / 2.0;
    let bottom = top + msgs.len() as f64 * row + 40.0;
    let mut nodes: Vec<Value> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            obj! {
                "id" => format!("p{}", i + 1), "kind" => "shape", "shape" => "ROUNDED_RECTANGLE", "role" => "participant",
                "x" => PAD + i as f64 * col_w, "y" => TITLE, "width" => head_w, "height" => head_h, "text" => p,
                "fill" => "#0B0D12", "textColor" => "#FFFFFF", "fontSize" => 16i64,
            }
        })
        .collect();
    let pt = |x: f64, y: f64| obj! { "x" => x, "y" => y };
    let mut edges: Vec<Value> = (0..parts.len())
        .map(|i| obj! { "id" => format!("life{}", i + 1), "fromPoint" => pt(cx(i), TITLE + head_h), "toPoint" => pt(cx(i), bottom), "dashed" => true, "color" => "#CBD5E1", "noArrow" => true })
        .collect();
    for (k, m) in msgs.iter().enumerate() {
        let mv = Some(m);
        let y = top + k as f64 * row + row / 2.0;
        let (a, b) = (col(js::get(mv, "from")).unwrap(), col(js::get(mv, "to")).unwrap());
        let kind = js::nullish(js::get(mv, "kind")).map_or("call".to_string(), js::string);
        let label = format!("{}. {}", k + 1, js::to_string(js::get(mv, "text")));
        if a == b {
            // a step inside one participant: a short hook to the right
            edges.push(obj! { "id" => format!("m{}", k + 1), "fromPoint" => pt(cx(a), y - 12.0), "toPoint" => pt(cx(a) + 48.0, y + 12.0), "label" => label, "lineType" => "ELBOWED", "color" => "#334155" });
            continue;
        }
        edges.push(obj! {
            "id" => format!("m{}", k + 1), "fromPoint" => pt(cx(a), y), "toPoint" => pt(cx(b), y), "label" => label,
            "dashed" => if kind == "reply" { Some(Value::Bool(true)) } else { None }, "lineType" => "STRAIGHT",
            "color" => if kind == "reply" { "#64748B" } else { "#2563EB" },
        });
    }
    let width = PAD + (parts.len() - 1) as f64 * col_w + head_w + PAD;
    let notes = notes_of(spec)?;
    nodes.extend(place_notes(&notes, width + 24.0, TITLE)?);
    Ok(obj! {
        "section" => section(spec, "Sequence · System design · Draft", notes.len(), width, bottom),
        "lanes" => Value::Array(vec![]), "nodes" => Value::Array(nodes), "edges" => Value::Array(edges),
    })
}

/// `boardText(map)`: board_map → plain text, one line per item, connectors as "a → b [label]".
pub fn board_text(map: &Value) -> Result<String> {
    let mp = Some(map);
    let sections = js::get(mp, "sections").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let loose = js::get(mp, "loose").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let mut lines = vec![format!("Board \"{}\" ({})", js::to_string(js::get(mp, "file")), js::to_string(js::get(mp, "editorType")))];
    let items_of = |s: &Value| js::get(Some(s), "items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    let mut name_of: Vec<(Value, Value)> = vec![];
    for it in sections.iter().flat_map(items_of).chain(loose.iter().cloned()) {
        let iv = Some(&it);
        if js::get(iv, "kind") != Some(&Value::String("connector".into())) {
            let id = js::get(iv, "id").cloned().unwrap_or(Value::Null);
            let name = js::nullish(js::get(iv, "waraId")).or(js::get(iv, "id")).cloned().unwrap_or(Value::Null);
            match name_of.iter_mut().find(|(k, _)| js::strict_eq(Some(k), Some(&id))) {
                Some(e) => e.1 = name,
                None => name_of.push((id, name)),
            }
        }
    }
    let lookup = |v: Option<&Value>| name_of.iter().find(|(k, _)| js::strict_eq(Some(k), v)).map(|(_, n)| n.clone());
    let item = |it: &Value, lanes: Option<&Vec<Value>>| -> String {
        let iv = Some(it);
        let kind = js::get(iv, "kind");
        if kind == Some(&Value::String("connector".into())) {
            let end = |k: &str| match js::nullish(lookup(js::get(iv, k)).as_ref()) {
                Some(n) => js::string(n),
                None => js::nullish(js::get(iv, k)).map_or("?".to_string(), js::string),
            };
            let label = js::get(iv, "label");
            return format!(
                "  {} → {}{}{}",
                end("from"),
                end("to"),
                if js::truthy(label) { format!(" [{}]", js::to_string(label)) } else { String::new() },
                if js::truthy(js::get(iv, "dashed")) { " (dashed)" } else { "" }
            );
        }
        // inside a lane band both ways (stickies beside the diagram belong to no lane)
        let n = |k: &str| js::to_number(js::get(iv, k));
        let cx = n("x") + n("w") / 2.0;
        let cy = n("y") + n("h") / 2.0;
        let lane = lanes.and_then(|ls| {
            ls.iter().find(|l| {
                let lv = Some(*l);
                let ln = |k: &str| js::to_number(js::get(lv, k));
                cy >= ln("y") && cy < ln("y") + ln("h") && (js::nullish(js::get(lv, "w")).is_none() || (cx >= ln("x") && cx < ln("x") + ln("w")))
            })
        });
        let lane_name = lane.and_then(|l| js::get(Some(l), "name"));
        let tag = match kind.and_then(|k| k.as_str()) {
            Some("sticky") => {
                let c = js::nullish(js::get(iv, "role")).or_else(|| js::nullish(js::get(iv, "color"))).map(js::string).unwrap_or_default();
                js::trim(&format!("sticky {c}")).to_string()
            }
            Some("shape") => match js::nullish(js::get(iv, "role")) {
                Some(r) => js::string(r),
                None => match js::nullish(js::get(iv, "shape")) {
                    Some(s) => js::string(s).to_lowercase(),
                    None => "undefined".into(),
                },
            },
            _ => js::to_string(kind),
        };
        let at = if js::truthy(lane_name) { format!(" @{}", js::to_string(lane_name)) } else { String::new() };
        let text = js::nullish(js::get(iv, "text")).cloned().unwrap_or_else(|| Value::String(String::new()));
        format!("  {} · {tag}{at} · {}", js::to_string(js::nullish(js::get(iv, "waraId")).or(js::get(iv, "id"))), js::stringify(&text))
    };
    for s in &sections {
        let sv = Some(s);
        let lanes = js::get(sv, "lanes").and_then(|l| l.as_array());
        let lane_list = match lanes {
            Some(l) if !l.is_empty() => format!(" · lanes: {}", js::join(&l.iter().map(|l| js::get(Some(l), "name").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(), " | ")),
            _ => String::new(),
        };
        lines.push(String::new());
        lines.push(format!("## {} ({}){lane_list}", js::to_string(js::get(sv, "name")), js::to_string(js::get(sv, "id"))));
        let items = items_of(s);
        let mut shapes: Vec<&Value> = items.iter().filter(|i| js::get(Some(i), "kind") != Some(&Value::String("connector".into()))).collect();
        shapes.sort_by(|a, b| {
            let g = |v: &Value, k: &str| js::to_number(js::get(Some(v), k));
            let dx = g(a, "x") - g(b, "x");
            js::cmp_num(if dx != 0.0 && !dx.is_nan() { dx } else { g(a, "y") - g(b, "y") })
        });
        for it in shapes {
            lines.push(item(it, lanes));
        }
        for it in items.iter().filter(|i| js::get(Some(i), "kind") == Some(&Value::String("connector".into()))) {
            lines.push(item(it, None));
        }
    }
    if !loose.is_empty() {
        lines.push(String::new());
        lines.push("## Outside sections".into());
        for it in &loose {
            lines.push(item(it, None));
        }
    }
    Ok(lines.join("\n"))
}
