//! Port of `legacy-node/motion-handoff.mjs`: Figma Motion → handoff — which animated layers become code
//! (+ motion tokens) and which become a Lottie, lint for UI motion, the token set the code should use, and
//! code snippets (CSS / Framer Motion). Input is always a motion_read result (with hints).

use crate::js::{self, num, r3};
use crate::motion_figma::{node_tracks, SpecKey, SpecTrack};
use crate::obj;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

// ---------------------------------------------------------------- classify
const LOTTIE_ONLY: [&str; 3] = ["draw", "fill", "stroke"]; // path drawing / colour morphs on art read better as Lottie

/// `motionContext(sectionName)`: "landing" (landing / product / brand pages, storyboards) or "ui".
pub fn motion_context(section_name: Option<&Value>) -> &'static str {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"(?i)storyboard|landing|marketing|brand").unwrap());
    let s = js::nullish(section_name).map(js::string).unwrap_or_default();
    if re.is_match(&s) {
        "landing"
    } else {
        "ui"
    }
}

/// One animated layer and where its motion goes.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: Option<Value>,
    pub name: Option<Value>,
    pub parent_id: Option<Value>,
    pub nth: Option<Value>,
    pub to: String, // "code" | "lottie"
    pub why: String,
    pub props: Vec<String>,
    pub span: [f64; 2],
    pub tracks: Vec<SpecTrack>,
    pub styles: Vec<Option<Value>>,
}

impl Row {
    /// The same JSON shape as the JS row.
    pub fn to_json(&self) -> Value {
        obj! {
            "id" => self.id.clone(), "name" => self.name.clone(), "parentId" => self.parent_id.clone(), "nth" => self.nth.clone(),
            "to" => self.to.as_str(), "why" => self.why.as_str(),
            "props" => Value::Array(self.props.iter().map(|p| Value::String(p.clone())).collect()),
            "span" => js::nums(&self.span),
            "tracks" => Value::Array(self.tracks.iter().map(|t| t.to_json()).collect()),
            "styles" => Value::Array(self.styles.iter().map(|s| s.clone().unwrap_or(Value::Null)).collect()),
        }
    }
    fn name_s(&self) -> String {
        js::to_string(self.name.as_ref())
    }
    fn is_root(&self, r: &Value) -> bool {
        js::strict_eq(self.id.as_ref(), js::get(js::get(Some(r), "root"), "id"))
    }
}

/// `for (const n of r.animated)`: V8 names the expression when it is not iterable.
fn animated(r: &Value) -> Result<&Vec<Value>> {
    match js::get(Some(r), "animated") {
        Some(Value::Array(a)) => Ok(a),
        _ => Err(anyhow!("r.animated is not iterable")),
    }
}

fn len_of(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Array(a)) => a.len(),
        _ => 0,
    }
}

/// `classify(r, warnings, { context })` → rows `{ id, name, parentId, to: "code"|"lottie", why, props, span, tracks }`.
pub fn classify(r: &Value, warnings: &mut Vec<String>, context: &str) -> Result<Vec<Row>> {
    let landing = context == "landing";
    let root_id = js::get(js::get(Some(r), "root"), "id");
    let mut rows = vec![];
    for n in animated(r)? {
        let nv = Some(n);
        if len_of(js::get(nv, "tracks")) == 0 && len_of(js::get(nv, "styles")) == 0 {
            continue;
        }
        let tracks = node_tracks(n, warnings)?;
        let mut props: Vec<String> = vec![];
        for t in &tracks {
            if !props.contains(&t.prop) {
                props.push(t.prop.clone());
            }
        }
        let times: Vec<f64> = tracks.iter().flat_map(|t| t.keys.iter().map(|k| k.ms)).collect();
        let span = if times.is_empty() { [0.0, 0.0] } else { [js::min_of(times.iter().copied()), js::max_of(times.iter().copied())] };
        let h = js::nullish(js::get(nv, "hint"));
        let hg = |k: &str| js::get(h, k);
        let complex = tracks.iter().any(|t| t.keys.len() > 2) || props.iter().any(|p| LOTTIE_ONLY.contains(&p.as_str()));
        let component = hg("component");
        let (to, why): (&str, String) = if matches!(component, Some(Value::String(c)) if c.starts_with("Lottie/")) {
            ("lottie", format!("component {}", js::to_string(component)))
        } else if js::strict_eq(js::get(nv, "id"), root_id) {
            ("code", "the screen/frame itself — a transition".into())
        } else if js::truthy(hg("text")) {
            ("code", "has text (must stay real, translatable text)".into())
        } else if js::truthy(component) || js::truthy(hg("instances")) {
            ("code", format!("UI component{}", if js::truthy(component) { format!(" {}", js::to_string(component)) } else { String::new() }))
        } else if js::truthy(hg("autoLayout")) {
            ("code", "auto-layout container (UI)".into())
        } else if js::truthy(hg("image")) {
            ("code", "image — CSS moves it fine".into())
        } else if landing && js::truthy(hg("vectorOnly")) {
            ("code", "part of the scroll scene — inline SVG driven by GSAP / ScrollTrigger".into())
        } else if js::truthy(hg("vectorOnly")) && complex {
            ("lottie", "vector art with path/colour/multi-step motion".into())
        } else if js::truthy(hg("vectorOnly")) {
            ("code", "vector with a simple transform — CSS is enough".into())
        } else {
            ("code", "UI layer".into())
        };
        let styles = match js::get(nv, "styles") {
            Some(Value::Array(s)) => s.iter().map(|s| js::get(Some(s), "name").cloned()).collect(),
            _ => vec![],
        };
        rows.push(Row {
            id: js::get(nv, "id").cloned(),
            name: js::get(nv, "name").cloned(),
            parent_id: js::get(nv, "parentId").cloned(),
            nth: js::get(nv, "nth").cloned(),
            to: to.into(),
            why,
            props,
            span,
            tracks,
            styles,
        });
    }
    // Vector pieces animating together under one parent are one illustration: if any is Lottie, all are.
    let lottie_parents: Vec<Option<Value>> =
        if landing { vec![] } else { rows.iter().filter(|x| x.to == "lottie" && !x.why.starts_with("component")).map(|x| x.parent_id.clone()).collect() };
    let all = animated(r)?;
    for x in &mut rows {
        let h = all.iter().find(|n| js::strict_eq(js::get(Some(n), "id"), x.id.as_ref())).and_then(|n| js::nullish(js::get(Some(n), "hint")));
        let vector_only = js::truthy(js::get(h, "vectorOnly"));
        let in_parent = lottie_parents.iter().any(|p| js::strict_eq(p.as_ref(), x.parent_id.as_ref()));
        if x.to == "code" && vector_only && in_parent && !js::strict_eq(x.id.as_ref(), root_id) {
            x.to = "lottie".into();
            x.why = "part of the same illustration".into();
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------- lint (UI motion that ships as code)
/// JS `a < b` for two key values.
fn js_lt(a: Option<&Value>, b: Option<&Value>) -> bool {
    let prim = |v: Option<&Value>| -> Option<Value> {
        match v {
            Some(Value::Array(_)) | Some(Value::Object(_)) => Some(Value::String(js::to_string(v))),
            other => other.cloned(),
        }
    };
    match (prim(a), prim(b)) {
        (Some(Value::String(x)), Some(Value::String(y))) => x.encode_utf16().lt(y.encode_utf16()),
        (x, y) => js::to_number(x.as_ref()) < js::to_number(y.as_ref()),
    }
}

/// `lint(rows, r, { context })`.
pub fn lint(rows: &[Row], r: &Value, context: &str) -> Vec<String> {
    let mut out = vec![];
    let landing = context == "landing";
    let code: Vec<&Row> = rows.iter().filter(|x| x.to == "code").collect();
    for x in &code {
        let len = x.span[1] - x.span[0];
        if !landing && len > 500.0 && !x.is_root(r) {
            out.push(format!("{}: {}ms of UI motion feels slow — keep UI moves 150–400ms", x.name_s(), js::num_str(len)));
        }
        if x.props.iter().any(|p| p == "width" || p == "height") {
            out.push(format!("{}: animates width/height (re-layout every frame) — use scale, or a layout animation in code", x.name_s()));
        }
        if x.tracks.iter().any(|t| t.keys.iter().any(|k| matches!(k.ease.as_deref(), Some("spring") | Some("anticipate")))) {
            out.push(format!("{}: bouncy easing on UI — fine for a playful taste, check it matches the project's motion taste", x.name_s()));
        }
    }
    let last_end = js::max_of(std::iter::once(0.0).chain(code.iter().filter(|x| !x.is_root(r)).map(|x| x.span[1])));
    if !landing && last_end > 1000.0 {
        out.push(format!("the screen keeps moving until {}ms — content should be usable within ~1s (shorten the stagger)", js::num_str(last_end)));
    }
    // landing: a moment can be long, but when it plays on time (load) it must not hold the reader
    if landing && last_end > 3000.0 {
        out.push(format!("this moment runs {}ms — on load keep it under ~2s; longer stories belong on scroll (pinned + scrub)", js::num_str(last_end)));
    }
    let (durations, easings) = collect(&code);
    if durations.len() > 4 {
        out.push(format!("{} different durations ({}ms) — snap to 3–4 tokens", durations.len(), durations.iter().map(|d| js::num_str(*d)).collect::<Vec<_>>().join(", ")));
    }
    if easings.len() > if landing { 4 } else { 3 } {
        out.push(format!("{} different easings ({}) — a project usually needs 2–3", easings.len(), easings.join(", ")));
    }
    let mut mixed: Vec<Option<Value>> = vec![];
    for x in rows.iter().filter(|x| x.to == "lottie") {
        if !mixed.iter().any(|p| js::strict_eq(p.as_ref(), x.parent_id.as_ref())) {
            mixed.push(x.parent_id.clone());
        }
    }
    for p in &mixed {
        if rows.iter().any(|x| x.to == "code" && js::strict_eq(x.parent_id.as_ref(), p.as_ref())) {
            out.push(format!("frame {} mixes Lottie art and UI layers — group the art into its own frame and export that frame with motion_to_lottie", js::to_string(p.as_ref())));
        }
    }
    let smell: Vec<&Row> = rows.iter().filter(|x| !x.is_root(r) && !x.tracks.is_empty()).collect();
    out.extend(ai_smell(&smell));
    out
}

// The tells of generated motion (motion-craft §6): every layer the same, no leader, wrong curves.
fn ai_smell(rows: &[&Row]) -> Vec<String> {
    let mut out = vec![];
    let tag = |s: String| format!("AI smell: {s} (motion-craft)");
    let first = |x: &Row, prop: &str| -> Option<Vec<SpecKey>> { x.tracks.iter().find(|t| t.prop == prop).map(|t| t.keys.clone()) };
    let len = |x: &Row| x.span[1] - x.span[0];
    let late: Vec<&&Row> = rows
        .iter()
        .filter(|x| {
            let Some(op) = first(x, "opacity") else { return false };
            let (Some(f), Some(l)) = (op.first(), op.last()) else { return false };
            js_lt(f.v.as_ref(), l.v.as_ref()) && matches!(f.ease.as_deref(), Some("in") | Some("linear") | Some("accelerate") | Some("back-in"))
        })
        .collect();
    if !late.is_empty() {
        let names: Vec<Value> = late.iter().map(|x| x.name.clone().unwrap_or(Value::Null)).collect();
        let e = first(late[0], "opacity").and_then(|k| k[0].ease.clone()).unwrap_or_else(|| "undefined".into());
        out.push(tag(format!("{} appear{} with {} — entrances ease out", js::join(&names, ", "), if late.len() == 1 { "s" } else { "" }, e)));
    }
    if rows.len() < 3 {
        return out;
    }
    let distinct = |xs: Vec<f64>| -> Vec<f64> {
        let mut d: Vec<f64> = vec![];
        for x in xs {
            if !d.iter().any(|y| *y == x || (y.is_nan() && x.is_nan())) {
                d.push(x);
            }
        }
        d
    };
    if distinct(rows.iter().map(|x| len(x)).collect()).len() == 1 {
        out.push(tag(format!("all {} layers last {}ms — give the leader more time and the followers less", rows.len(), js::num_str(len(rows[0])))));
    }
    let mut eases: Vec<String> = vec![];
    for x in rows {
        for t in &x.tracks {
            for k in &t.keys[..t.keys.len().saturating_sub(1)] {
                let e = k.ease.clone().unwrap_or_else(|| "standard".into());
                if !eases.contains(&e) {
                    eases.push(e);
                }
            }
        }
    }
    if eases.len() == 1 {
        out.push(tag(format!("one easing ({}) on every layer — shape the leader's curve differently", eases[0])));
    }
    if distinct(rows.iter().map(|x| x.span[0]).collect()).len() == 1 {
        out.push(tag("everything starts at once — let one element lead".into()));
    }
    // fade + rise of the same distance on everything
    let rise: Vec<Option<String>> = rows
        .iter()
        .map(|x| {
            let m = first(x, "move")?;
            let o = first(x, "opacity")?;
            let zero = matches!(o.first().and_then(|k| k.v.as_ref()), Some(Value::Number(n)) if js::number_f64(n) == 0.0);
            if !zero {
                return None;
            }
            m.first().and_then(|k| k.v.as_ref()).map(js::stringify)
        })
        .collect();
    if rise.iter().all(|r| r.as_ref().is_some_and(|s| !s.is_empty())) {
        let mut d: Vec<&String> = vec![];
        for r in rise.iter().flatten() {
            if !d.contains(&r) {
                d.push(r);
            }
        }
        if d.len() == 1 {
            out.push(tag(format!("every layer fades in and moves from {} — vary distance by weight, or move only the leader", d[0])));
        }
    }
    // the same offset between every start, over a long run
    let mut starts = distinct(rows.iter().map(|x| x.span[0]).collect());
    starts.sort_by(|a, b| js::cmp_num(a - b));
    let gaps = distinct(starts.windows(2).map(|w| w[1] - w[0]).collect());
    if starts.len() > 6 && gaps.len() == 1 {
        out.push(tag(format!("{} items staggered evenly — cap the stagger at ~5–6 items, the rest arrive together", starts.len())));
    }
    out
}

// ---------------------------------------------------------------- tokens
fn css_ease_named(e: &str) -> Option<&'static str> {
    Some(match e {
        "linear" => "linear",
        "in" => "cubic-bezier(0.42, 0, 1, 1)",
        "out" => "cubic-bezier(0, 0, 0.58, 1)",
        "in-out" => "cubic-bezier(0.42, 0, 0.58, 1)",
        "standard" => "cubic-bezier(0.2, 0, 0, 1)",
        "decelerate" => "cubic-bezier(0, 0, 0, 1)",
        "accelerate" => "cubic-bezier(0.3, 0, 1, 1)",
        "emphasized" => "cubic-bezier(0.05, 0.7, 0.1, 1)",
        "ease" => "ease",
        "back" => "cubic-bezier(0.34, 1.56, 0.64, 1)",
        "back-in" => "cubic-bezier(0.36, 0, 0.66, -0.56)",
        "anticipate" => "cubic-bezier(0.68, -0.6, 0.32, 1.6)",
        "spring" => "cubic-bezier(0.34, 1.56, 0.64, 1)",
        "hold" => "steps(1, end)",
        _ => return None,
    })
}

fn css_ease(e: Option<&str>) -> String {
    if let Some(c) = css_ease_named(e.unwrap_or("standard")) {
        return c.into();
    }
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"^cubic\(([^)]+)\)$").unwrap());
    match e.and_then(|e| re.captures(e)) {
        Some(m) => format!("cubic-bezier({})", m[1].split(',').collect::<Vec<_>>().join(", ")),
        None => "cubic-bezier(0.2, 0, 0, 1)".into(),
    }
}

fn ease_name(e: Option<&str>) -> Option<String> {
    match e {
        Some(s) if s.starts_with("cubic(") => None,
        Some(s) => Some(s.into()),
        None => Some("standard".into()),
    }
}

fn collect(code: &[&Row]) -> (Vec<f64>, Vec<String>) {
    let mut durations: Vec<f64> = vec![];
    let mut easings: Vec<String> = vec![];
    for x in code {
        if x.span[1] > x.span[0] {
            let d = x.span[1] - x.span[0];
            if !durations.contains(&d) {
                durations.push(d);
            }
        }
        for t in &x.tracks {
            for k in &t.keys[..t.keys.len().saturating_sub(1)] {
                let e = k.ease.clone().unwrap_or_else(|| "standard".into());
                if !easings.contains(&e) {
                    easings.push(e);
                }
            }
        }
    }
    (snap(durations), easings.into_iter().filter(|e| e != "hold").collect())
}

// merge durations closer than 30ms
fn snap(mut list: Vec<f64>) -> Vec<f64> {
    list.sort_by(|a, b| js::cmp_num(a - b));
    let mut out: Vec<f64> = vec![];
    for d in list {
        if out.is_empty() || d - out.last().unwrap() > 30.0 {
            out.push(d);
        }
    }
    out
}

const DURATION_NAMES: [&[&str]; 5] = [&["base"], &["fast", "base"], &["fast", "base", "slow"], &["fast", "base", "slow", "slower"], &["instant", "fast", "base", "slow", "slower"]];

/// The motion token set: `{ duration: { name: ms }, easing: { name: css }, css, json }`.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub duration: Vec<(String, f64)>,
    pub easing: Vec<(String, String)>,
    pub css: String,
    pub json: String,
}

impl Tokens {
    pub fn to_json(&self) -> Value {
        let mut d = serde_json::Map::new();
        for (k, v) in &self.duration {
            d.insert(k.clone(), num(*v));
        }
        let mut e = serde_json::Map::new();
        for (k, v) in &self.easing {
            e.insert(k.clone(), Value::String(v.clone()));
        }
        obj! { "duration" => Value::Object(d), "easing" => Value::Object(e), "css" => self.css.as_str(), "json" => self.json.as_str() }
    }
}

/// `tokens(rows)`: from the code-bound layers.
pub fn tokens(rows: &[Row]) -> Tokens {
    let code: Vec<&Row> = rows.iter().filter(|x| x.to == "code").collect();
    let (durations, easings) = collect(&code);
    let names: Vec<String> = match durations.len().checked_sub(1).and_then(|i| DURATION_NAMES.get(i)) {
        Some(n) => n.iter().map(|s| s.to_string()).collect(),
        None => (0..durations.len()).map(|i| format!("d{}", i + 1)).collect(),
    };
    let mut duration: Vec<(String, f64)> = vec![];
    for (i, d) in durations.iter().enumerate() {
        set(&mut duration, names[i].clone(), *d);
    }
    let mut custom = 0;
    let mut easing: Vec<(String, String)> = vec![];
    for e in &easings {
        let name = ease_name(Some(e)).unwrap_or_else(|| {
            custom += 1;
            format!("custom-{custom}")
        });
        set(&mut easing, name, css_ease(Some(e)));
    }
    let mut css = vec![":root {".to_string()];
    css.extend(duration.iter().map(|(k, v)| format!("  --motion-duration-{k}: {}ms;", js::num_str(*v))));
    css.extend(easing.iter().map(|(k, v)| format!("  --motion-easing-{k}: {v};")));
    css.push("}".into());
    let mut dj = serde_json::Map::new();
    for (k, v) in &duration {
        dj.insert(k.clone(), Value::String(format!("{}ms", js::num_str(*v))));
    }
    let mut ej = serde_json::Map::new();
    for (k, v) in &easing {
        ej.insert(k.clone(), Value::String(v.clone()));
    }
    let json = js::stringify_pretty(&obj! { "motion" => obj! { "duration" => Value::Object(dj), "easing" => Value::Object(ej) } }, 2);
    Tokens { duration, easing, css: css.join("\n"), json }
}

// Object.fromEntries: a repeated key keeps its first place and takes the last value
fn set<V>(list: &mut Vec<(String, V)>, k: String, v: V) {
    if let Some(e) = list.iter_mut().find(|(x, _)| *x == k) {
        e.1 = v;
    } else {
        list.push((k, v));
    }
}

fn duration_var(tk: &Tokens, ms: f64) -> String {
    match tk.duration.iter().find(|(_, v)| (v - ms).abs() <= 30.0) {
        Some((k, _)) => format!("var(--motion-duration-{k})"),
        None => format!("{}ms", js::num_str(ms)),
    }
}
fn easing_var(tk: &Tokens, e: Option<&str>) -> String {
    let css = css_ease(e);
    match tk.easing.iter().find(|(_, v)| *v == css) {
        Some((k, _)) => format!("var(--motion-easing-{k})"),
        None => css,
    }
}

// ---------------------------------------------------------------- code
/// A value in a code frame: a number, a list of numbers, or whatever the key held.
#[derive(Debug, Clone)]
enum HV {
    Num(f64),
    Arr(Vec<f64>),
    Raw(Value),
    Undef,
}

impl HV {
    fn from(v: Option<&Value>) -> HV {
        match v {
            Some(Value::Number(n)) => HV::Num(js::number_f64(n)),
            Some(v) => HV::Raw(v.clone()),
            None => HV::Undef,
        }
    }
    fn is_array(&self) -> bool {
        matches!(self, HV::Arr(_) | HV::Raw(Value::Array(_)))
    }
    fn number(&self) -> f64 {
        match self {
            HV::Num(x) => *x,
            HV::Arr(a) => js::str_to_number(&a.iter().map(|x| js::num_str(*x)).collect::<Vec<_>>().join(",")),
            HV::Raw(v) => js::number(v),
            HV::Undef => f64::NAN,
        }
    }
    fn idx(&self, i: usize) -> Option<HV> {
        match self {
            HV::Arr(a) => a.get(i).map(|x| HV::Num(*x)),
            HV::Raw(Value::Array(a)) => a.get(i).map(|x| HV::from(Some(x))),
            _ => None,
        }
    }
    fn len(&self) -> usize {
        match self {
            HV::Arr(a) => a.len(),
            HV::Raw(Value::Array(a)) => a.len(),
            _ => 0,
        }
    }
    fn truthy(&self) -> bool {
        match self {
            HV::Num(x) => *x != 0.0 && !x.is_nan(),
            HV::Arr(_) => true,
            HV::Raw(v) => js::truthy(Some(v)),
            HV::Undef => false,
        }
    }
    fn is_nullish(&self) -> bool {
        matches!(self, HV::Raw(Value::Null) | HV::Undef)
    }
    fn to_str(&self) -> String {
        match self {
            HV::Num(x) => js::num_str(*x),
            HV::Arr(a) => a.iter().map(|x| js::num_str(*x)).collect::<Vec<_>>().join(","),
            HV::Raw(v) => js::string(v),
            HV::Undef => "undefined".into(),
        }
    }
    fn is_number(&self) -> bool {
        matches!(self, HV::Num(_))
    }
    fn to_json(&self) -> Option<Value> {
        match self {
            HV::Num(x) => Some(num(*x)),
            HV::Arr(a) => Some(js::nums(a)),
            HV::Raw(v) => Some(v.clone()),
            HV::Undef => None,
        }
    }
    fn same(&self, o: &HV) -> bool {
        match (self, o) {
            (HV::Num(a), HV::Num(b)) => a == b,
            (HV::Raw(a), HV::Raw(b)) => js::strict_eq(Some(a), Some(b)),
            (HV::Undef, HV::Undef) => true,
            _ => false,
        }
    }
}

fn lerp(a: &HV, b: &HV, t: f64) -> HV {
    if a.is_array() {
        let n = a.len();
        return HV::Arr(
            (0..n)
                .map(|i| {
                    let x = a.idx(i).map_or(f64::NAN, |x| x.number());
                    let y = b.idx(i).map_or(f64::NAN, |y| y.number());
                    x + (y - x) * t
                })
                .collect(),
        );
    }
    if let HV::Num(x) = a {
        return HV::Num(x + (b.number() - x) * t);
    }
    if t < 1.0 {
        a.clone()
    } else {
        b.clone()
    }
}

fn at(keys: &[SpecKey], ms: f64) -> HV {
    if keys.is_empty() {
        return HV::Undef;
    }
    if ms <= keys[0].ms {
        return HV::from(keys[0].v.as_ref());
    }
    for i in 1..keys.len() {
        let (k0, k1) = (&keys[i - 1], &keys[i]);
        if ms <= k1.ms {
            let v0 = HV::from(k0.v.as_ref());
            return if k0.ease.as_deref() == Some("hold") && ms < k1.ms { v0 } else { lerp(&v0, &HV::from(k1.v.as_ref()), (ms - k0.ms) / (k1.ms - k0.ms)) };
        }
    }
    HV::from(keys.last().unwrap().v.as_ref())
}

fn slug(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"[^a-z0-9ก-๙]+").unwrap());
    let lower = s.to_lowercase();
    let d = re.replace_all(&lower, "-");
    let d = d.strip_prefix('-').unwrap_or(&d);
    let d = d.strip_suffix('-').unwrap_or(d);
    if d.is_empty() {
        "layer".into()
    } else {
        d.into()
    }
}

// vec(v): Array.isArray(v) ? v : [v, v]
fn vec_at(v: &HV, i: usize) -> Option<HV> {
    if v.is_array() {
        v.idx(i)
    } else if i < 2 {
        Some(v.clone())
    } else {
        None
    }
}

fn r3_of(v: Option<HV>) -> String {
    js::num_str(r3(v.map_or(f64::NAN, |v| v.number())))
}

fn css_frame(get: &dyn Fn(&str) -> Option<HV>) -> Vec<String> {
    let mut parts = vec![];
    let mut tf = vec![];
    let m = get("move");
    let s = get("scale");
    let rot = get("rotate");
    if let Some(m) = m.as_ref().filter(|m| m.truthy()) {
        tf.push(format!("translate({}px, {}px)", r3_of(vec_at(m, 0)), r3_of(vec_at(m, 1))));
    }
    if let Some(s) = s.as_ref().filter(|s| s.truthy()) {
        let (x, y) = (vec_at(s, 0), vec_at(s, 1));
        let same = match (&x, &y) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        };
        tf.push(if same { format!("scale({})", r3_of(x)) } else { format!("scale({}, {})", r3_of(x), r3_of(y)) });
    }
    if let Some(r) = rot.filter(|r| !r.is_nullish()) {
        tf.push(format!("rotate({}deg)", r3_of(Some(r))));
    }
    if !tf.is_empty() {
        parts.push(format!("transform: {}", tf.join(" ")));
    }
    let present = |p: &str| get(p).filter(|v| !v.is_nullish());
    if let Some(o) = present("opacity") {
        parts.push(format!("opacity: {}", r3_of(Some(o))));
    }
    if let Some(w) = present("width") {
        parts.push(format!("width: {}px", r3_of(Some(w))));
    }
    if let Some(h) = present("height") {
        parts.push(format!("height: {}px", r3_of(Some(h))));
    }
    if let Some(r) = present("radius") {
        parts.push(format!("border-radius: {}px", r3_of(Some(r))));
    }
    if let Some(f) = present("fill") {
        parts.push(format!("background-color: {}", f.to_str()));
    }
    parts
}

struct Group<'a> {
    x: &'a Row,
    count: usize,
    step: f64,
}

// Same-named layers with the same motion, started at even steps (a list appearing one by one) → one
// animation plus a per-item delay, instead of a copy per item.
fn stagger_groups(rows: &[Row]) -> Vec<Group<'_>> {
    let sig = |x: &Row| {
        js::stringify(&Value::Array(
            x.tracks
                .iter()
                .map(|t| {
                    Value::Array(vec![
                        Value::String(t.prop.clone()),
                        Value::Array(
                            t.keys
                                .iter()
                                .map(|k| Value::Array(vec![num(k.ms - x.span[0]), k.v.clone().unwrap_or(Value::Null), k.ease.clone().map_or(Value::Null, Value::String)]))
                                .collect(),
                        ),
                    ])
                })
                .collect(),
        ))
    };
    let mut groups: Vec<(Option<Value>, String, Vec<&Row>)> = vec![];
    for x in rows.iter().filter(|r| r.to == "code" && !r.tracks.is_empty()) {
        let sx = sig(x);
        match groups.iter_mut().find(|(n, s, _)| js::strict_eq(n.as_ref(), x.name.as_ref()) && *s == sx) {
            Some(g) => g.2.push(x),
            None => groups.push((x.name.clone(), sx, vec![x])),
        }
    }
    groups
        .into_iter()
        .flat_map(|(_, _, members)| {
            let starts: Vec<f64> = members.iter().map(|m| m.span[0]).collect();
            let step = if starts.len() > 1 { starts[1] - starts[0] } else { 0.0 };
            let even = starts.iter().enumerate().all(|(i, st)| (st - (starts[0] + step * i as f64)).abs() <= 1.0);
            if even {
                vec![Group { x: members[0], count: members.len(), step }]
            } else {
                members.into_iter().map(|x| Group { x, count: 1, step: 0.0 }).collect()
            }
        })
        .collect()
}

fn track_get(x: &Row, p: &str, ms: f64) -> Option<HV> {
    x.tracks.iter().find(|t| t.prop == p).map(|t| at(&t.keys, ms))
}

/// `toCss(rows, tk, warnings)`: CSS @keyframes + animation per code layer, using the token variables.
pub fn to_css(rows: &[Row], tk: &Tokens, warnings: &mut Vec<String>) -> String {
    let mut out = vec![tk.css.clone(), String::new()];
    for Group { x, count, step } in stagger_groups(rows) {
        let nth = js::to_number(x.nth.as_ref());
        let name = slug(&x.name_s()) + &if nth > 1.0 && count == 1 { format!("-{}", js::to_string(x.nth.as_ref())) } else { String::new() };
        let [t0, t1] = x.span;
        let len = js::max(1.0, t1 - t0);
        let mut times: Vec<f64> = vec![];
        for t in &x.tracks {
            for k in &t.keys {
                if !times.contains(&k.ms) {
                    times.push(k.ms);
                }
            }
        }
        times.sort_by(|a, b| js::cmp_num(a - b));
        let mut key_times: Vec<String> = vec![];
        for t in &x.tracks {
            let s = t.keys.iter().map(|k| js::num_str(k.ms)).collect::<Vec<_>>().join(",");
            if !key_times.contains(&s) {
                key_times.push(s);
            }
        }
        if key_times.len() > 1 {
            warnings.push(format!("{}: props have keys at different times — one @keyframes blends them linearly between keys; Framer Motion output keeps them exact", x.name_s()));
        }
        out.push(format!(
            "/* {} — {}{} */",
            x.name_s(),
            x.why,
            if count > 1 { format!(" · {count} items, {}ms apart: set style=\"--i: 0\", 1, 2… on each", js::num_str(step)) } else { String::new() }
        ));
        out.push(format!("@keyframes {name} {{"));
        for &ms in &times {
            let ease = x.tracks.iter().filter_map(|t| t.keys.iter().find(|k| k.ms == ms)).find(|k| k.ease.as_deref().is_some_and(|e| !e.is_empty())).and_then(|k| k.ease.clone());
            let mut parts = css_frame(&|p: &str| track_get(x, p, ms));
            if ms < t1 {
                if let Some(e) = &ease {
                    parts.push(format!("animation-timing-function: {}", easing_var(tk, Some(e))));
                }
            }
            out.push(format!("  {}% {{ {}; }}", js::num_str(r3(((ms - t0) / len) * 100.0)), parts.join("; ")));
        }
        let delay = if count > 1 {
            format!("calc({}ms + var(--i, 0) * {}ms) ", js::num_str(t0), js::num_str(step))
        } else if t0 != 0.0 && !t0.is_nan() {
            format!("{}ms ", js::num_str(t0))
        } else {
            String::new()
        };
        out.push("}".into());
        out.push(format!(".{name} {{"));
        out.push(format!("  animation: {name} {} {delay}both;", duration_var(tk, len)));
        out.push("}".into());
        out.push("@media (prefers-reduced-motion: reduce) {".into());
        out.push(format!("  .{name} {{ animation: none; }}"));
        out.push("}".into());
        out.push(String::new());
    }
    out.join("\n")
}

fn fm_ease(e: Option<&str>) -> String {
    let named = match e {
        Some("linear") => Some("linear"),
        Some("in") => Some("easeIn"),
        Some("out") => Some("easeOut"),
        Some("in-out") => Some("easeInOut"),
        Some("back") => Some("backOut"),
        Some("back-in") => Some("backIn"),
        Some("anticipate") => Some("anticipate"),
        _ => None,
    };
    if let Some(n) = named {
        return js::quote(n);
    }
    let css = css_ease(e);
    match css.strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')).filter(|inner| !inner.is_empty() && !inner.contains(')')) {
        Some(inner) => format!("[{inner}]"),
        None => "\"easeOut\"".into(),
    }
}

/// `toFramer(rows, tk)`: Framer Motion — one motion element per code layer, a transition per prop.
pub fn to_framer(rows: &[Row], tk: &Tokens) -> String {
    let mut out = vec![
        format!("// Motion tokens: {}", tk.duration.iter().map(|(k, v)| format!("{k} {}ms", js::num_str(*v))).collect::<Vec<_>>().join(", ")),
        "// Respect reduced motion: wrap the app in <MotionConfig reducedMotion=\"user\">".to_string(),
        String::new(),
    ];
    for Group { x, count, step } in stagger_groups(rows) {
        let mut animate = vec![];
        let mut transition = vec![];
        let delay_of = |t0: f64| if count > 1 { format!("{} + i * {}", js::num_str(r3(t0 / 1000.0)), js::num_str(r3(step / 1000.0))) } else { js::num_str(r3(t0 / 1000.0)) };
        for t in &x.tracks {
            // hold = stay, then jump
            let mut keys: Vec<(f64, HV, Option<String>)> = vec![];
            for (i, k) in t.keys.iter().enumerate() {
                keys.push((k.ms, HV::from(k.v.as_ref()), k.ease.clone()));
                if k.ease.as_deref() == Some("hold") {
                    if let Some(next) = t.keys.get(i + 1) {
                        keys.push((next.ms - 1.0, HV::from(k.v.as_ref()), None));
                    }
                }
            }
            if keys.is_empty() {
                continue;
            }
            let t0 = keys[0].0;
            let t1 = keys.last().unwrap().0;
            let len = js::max(1.0, t1 - t0);
            type Getter = fn(&HV) -> Option<HV>;
            let fields: Vec<(String, Getter)> = if t.prop == "move" {
                vec![("x".into(), |v| vec_at(v, 0)), ("y".into(), |v| vec_at(v, 1))]
            } else if t.prop == "scale" {
                let uniform = keys.iter().all(|k| match (vec_at(&k.1, 0), vec_at(&k.1, 1)) {
                    (Some(a), Some(b)) => a.same(&b),
                    (None, None) => true,
                    _ => false,
                });
                if uniform {
                    vec![("scale".into(), |v| vec_at(v, 0))]
                } else {
                    vec![("scaleX".into(), |v| vec_at(v, 0)), ("scaleY".into(), |v| vec_at(v, 1))]
                }
            } else {
                let f = match t.prop.as_str() {
                    "fill" => "backgroundColor".to_string(),
                    "radius" => "borderRadius".to_string(),
                    p => p.to_string(),
                };
                vec![(f, |v| Some(v.clone()))]
            };
            let n_fields = fields.len();
            for (field, get) in &fields {
                if n_fields > 1 {
                    let first = get(&keys[0].1);
                    let all_same = keys.iter().all(|k| match (get(&k.1), &first) {
                        (Some(a), Some(b)) => a.same(b),
                        (None, None) => true,
                        _ => false,
                    });
                    if all_same {
                        continue; // an axis that never moves
                    }
                }
                let vals: Vec<String> = keys
                    .iter()
                    .map(|k| match get(&k.1) {
                        Some(v) if v.is_number() => js::stringify(&num(r3(v.number()))),
                        Some(v) => v.to_json().map(|j| js::stringify(&j)).unwrap_or_default(),
                        None => String::new(),
                    })
                    .collect();
                animate.push(format!("{field}: [{}]", vals.join(", ")));
                let times: Vec<String> = keys.iter().map(|k| js::num_str(r3((k.0 - t0) / len))).collect();
                let eases: Vec<String> = keys[..keys.len() - 1].iter().map(|k| if k.2.as_deref() == Some("hold") { "\"linear\"".to_string() } else { fm_ease(k.2.as_deref()) }).collect();
                transition.push(format!(
                    "{field}: {{ duration: {}, delay: {}, times: [{}], ease: [{}] }}",
                    js::num_str(r3(len / 1000.0)),
                    delay_of(t0),
                    times.join(", "),
                    eases.join(", ")
                ));
            }
        }
        let el = [
            format!("<motion.div{}", if count > 1 { " key={item.id}" } else { "" }),
            format!("  animate={{{{ {} }}}}", animate.join(", ")),
            format!("  transition={{{{ {} }}}}", transition.join(",\n    ")),
            "/>".to_string(),
        ];
        out.push(format!("{{/* {} — {}{} */}}", x.name_s(), x.why, if count > 1 { format!(" · {count} items, {}ms apart", js::num_str(step)) } else { String::new() }));
        if count > 1 {
            out.push("{items.map((item, i) => (".into());
            out.extend(el.iter().map(|l| format!("  {l}")));
            out.push("))}".into());
        } else {
            out.extend(el);
        }
        out.push(String::new());
    }
    out.join("\n")
}

// ---------------------------------------------------------------- report
/// `checkText(r, rows, lints, context)`: the handoff table to show the user.
pub fn check_text(r: &Value, rows: &[Row], lints: &[String], context: &str) -> String {
    let root = js::get(Some(r), "root");
    let root_id = js::get(root, "id");
    let mut lines = vec![
        format!(
            "Motion handoff · {} ({}) · {} animated layer(s) · context {context}{}",
            js::to_string(js::get(root, "name")),
            js::to_string(root_id),
            rows.len(),
            if context == "landing" { " (motion-landing rules)" } else { "" }
        ),
        String::new(),
        "| layer | animates | → | why |".into(),
        "|---|---|---|---|".into(),
    ];
    for x in rows {
        let props = x.props.join(", ");
        let styles: Vec<Value> = x.styles.iter().map(|s| s.clone().unwrap_or(Value::Null)).collect();
        let what = if props.is_empty() { js::join(&styles, ", ") } else { props };
        let span = if x.span[1] != 0.0 && !x.span[1].is_nan() { format!("{}–{}ms", js::num_str(x.span[0]), js::num_str(x.span[1])) } else { String::new() };
        lines.push(format!(
            "| {}{} ({}) | {what} {span} | **{}** | {} |",
            x.name_s(),
            if js::to_number(x.nth.as_ref()) > 1.0 { format!(" #{}", js::to_string(x.nth.as_ref())) } else { String::new() },
            js::to_string(x.id.as_ref()),
            if x.to == "code" { "code + tokens" } else { "Lottie" },
            x.why
        ));
    }
    let lottie: Vec<&Row> = rows.iter().filter(|x| x.to == "lottie").collect();
    if !lottie.is_empty() {
        let mut parents: Vec<Option<Value>> = vec![];
        for x in &lottie {
            if !parents.iter().any(|p| js::strict_eq(p.as_ref(), x.parent_id.as_ref())) {
                parents.push(x.parent_id.clone());
            }
        }
        let shown: Vec<Value> = parents
            .iter()
            .map(|p| {
                if js::strict_eq(p.as_ref(), root_id) {
                    Value::String(format!("{} ({})", js::to_string(js::get(root, "name")), js::to_string(p.as_ref())))
                } else {
                    p.clone().unwrap_or(Value::Null)
                }
            })
            .collect();
        lines.push(String::new());
        lines.push(format!(
            "Lottie: motion_to_lottie on {}{}",
            js::join(&shown, ", "),
            if lottie.iter().any(|x| x.why.starts_with("component")) { " · Lottie/… components: export their main component's frame" } else { "" }
        ));
    }
    if rows.iter().any(|x| x.to == "code") {
        lines.push("Code: motion_tokens for the token set, motion_to_code for the snippets.".into());
    }
    if !lints.is_empty() {
        lines.push(String::new());
        lines.push("## Check".into());
        lines.extend(lints.iter().map(|l| format!("- {l}")));
    }
    lines.push(String::new());
    lines.push("Show this table to the user and wait for approval before writing code or files.".into());
    lines.join("\n")
}
