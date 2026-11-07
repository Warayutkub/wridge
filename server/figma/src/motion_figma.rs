//! Port of `legacy-node/motion-figma.mjs`: Figma Motion (beta Plugin API) on the server side — turning what
//! the plugin reads into compact text, and turning the short motion spec (the same one create_lottie takes)
//! into Figma keyframe tracks / presets.

use crate::js::{self, num, Obj};
use crate::obj;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;
use std::f64::consts::PI;
use std::sync::OnceLock;

fn cubic_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^cubic\(([^)]+)\)$").unwrap())
}

/// `ms(s)`: seconds → whole milliseconds.
pub fn ms(s: Option<&Value>) -> f64 {
    js::round(js::to_number(s) * 1000.0)
}

// val(v): arrays as [a,b], strings as they are, the rest as JSON
fn val(v: Option<&Value>) -> String {
    match v {
        Some(Value::Array(a)) => format!("[{}]", js::join(a, ",")),
        Some(Value::String(s)) => s.clone(),
        Some(v) => js::stringify(v),
        None => "undefined".into(),
    }
}

fn arr<'a>(v: Option<&'a Value>, what: &str) -> Result<&'a Vec<Value>> {
    match v {
        Some(Value::Array(a)) => Ok(a),
        other => Err(anyhow!("Cannot read properties of {} (reading '{what}')", js::to_string(other))),
    }
}

/// `Object.keys(v)` for a props value.
fn keys_of(v: &Value) -> Vec<String> {
    match v {
        Value::Object(o) => js::keys(o).into_iter().cloned().collect(),
        Value::Array(a) => (0..a.len()).map(|i| i.to_string()).collect(),
        Value::String(s) => (0..js::utf16_len(s)).map(|i| i.to_string()).collect(),
        _ => vec![],
    }
}

/// `motionText(r)`: motion_read result → one line per animated field ("OPACITY base 1 · 0ms 0 → 300ms 1 EASE_OUT").
pub fn motion_text(r: &Value) -> Result<String> {
    let r = Some(r);
    let root = js::get(r, "root");
    let timelines = js::get(r, "timelines");
    let tl = match timelines {
        Some(Value::Array(t)) if !t.is_empty() => format!(" · timeline {}", t.iter().map(|t| format!("{}ms", js::num_str(ms(js::get(Some(t), "duration"))))).collect::<Vec<_>>().join(", ")),
        _ => " · no timeline".into(),
    };
    let playhead = match js::nullish(js::get(r, "playhead")) {
        Some(p) => format!(" · playhead {}ms", js::num_str(ms(Some(p)))),
        None => String::new(),
    };
    let mut lines = vec![format!("Motion · {} ({}){tl}{playhead}", js::to_string(js::get(root, "name")), js::to_string(js::get(root, "id")))];
    let animated = arr(js::get(r, "animated"), "length")?;
    if animated.is_empty() {
        lines.push("Nothing animated in this frame yet.".into());
    }
    for n in animated {
        let n = Some(n);
        lines.push(String::new());
        lines.push(format!("{} ({}, {})", js::to_string(js::get(n, "name")), js::to_string(js::get(n, "id")), js::to_string(js::get(n, "type"))));
        for s in arr(js::get(n, "styles"), "length")? {
            let s = Some(s);
            let props = js::get(s, "props");
            lines.push(format!("  style {}{}", js::to_string(js::get(s, "name")), if js::truthy(props) { format!(" {}", js::stringify(props.unwrap())) } else { String::new() }));
        }
        for f in arr(js::get(n, "tracks"), "length")? {
            let f = Some(f);
            let field = js::to_string(js::get(f, "field"));
            for t in arr(js::get(f, "tracks"), "length")? {
                let t = Some(t);
                let keys = arr(js::get(t, "keys"), "map")?
                    .iter()
                    .map(|k| {
                        let k = Some(k);
                        let ease = js::get(k, "ease");
                        let e = if js::truthy(ease) && ease != Some(&Value::String("LINEAR".into())) {
                            format!(" {}", match ease {
                                Some(Value::String(s)) => s.clone(),
                                Some(v) => js::stringify(v),
                                None => String::new(),
                            })
                        } else {
                            String::new()
                        };
                        format!("{}ms {}{e}", js::num_str(ms(js::get(k, "t"))), val(js::get(k, "v")))
                    })
                    .collect::<Vec<_>>()
                    .join(" → ");
                // translation keys are offsets from the layer's own position, which Figma reports as the base
                let base = if field.starts_with("TRANSLATION") { format!("at {}, offset", val(js::get(f, "base"))) } else { format!("base {}", val(js::get(f, "base"))) };
                let op = js::get(t, "op");
                let op_s = if op == Some(&Value::String("SET".into())) {
                    String::new()
                } else {
                    match op {
                        Some(Value::String(s)) => format!(" ({})", s.to_lowercase()),
                        other => return Err(anyhow!("Cannot read properties of {} (reading 'toLowerCase')", js::to_string(other))),
                    }
                };
                lines.push(format!("  {field}{op_s} {base} · {keys}"));
            }
        }
    }
    if let Some(styles) = js::get(r, "animationStyles").filter(|v| js::truthy(Some(v))) {
        let list = styles.as_array().cloned().unwrap_or_default();
        lines.push(String::new());
        let count = match styles {
            Value::Array(a) => js::num_str(a.len() as f64),
            Value::String(s) => js::num_str(js::utf16_len(s) as f64),
            other => js::to_string(js::get(Some(other), "length")),
        };
        lines.push(format!("## Animation styles in this file ({count})"));
        for s in &list {
            let s = Some(s);
            let props = js::get(s, "props");
            let desc = js::get(s, "description");
            lines.push(format!(
                "  {} ({}){}{}",
                js::to_string(js::get(s, "name")),
                js::to_string(js::get(s, "styleId")),
                if js::truthy(props) { format!(" props: {}", keys_of(props.unwrap()).join(", ")) } else { String::new() },
                if js::truthy(desc) { format!(" — {}", js::to_string(desc).split('\n').next().unwrap_or("")) } else { String::new() },
            ));
        }
    }
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------- spec → Figma Motion items
/// UNITS: how our spec values map to what Figma stores.
pub fn rotate_to_figma(deg: f64) -> f64 {
    (deg * PI) / 180.0 // ROTATION: radians
}

/// `MOTION_PROPS`.
pub const MOTION_PROPS: [&str; 11] = ["opacity", "move", "x", "y", "scale", "rotate", "draw", "width", "height", "radius", "fill"];
const EASE_KEYS: [&str; 13] = ["linear", "in", "out", "in-out", "back", "back-in", "back-in-out", "spring", "bounce", "gentle", "quick", "slow", "hold"];
const BEZIER: [(&str, [f64; 4]); 6] = [
    ("standard", [0.2, 0.0, 0.0, 1.0]),
    ("decelerate", [0.0, 0.0, 0.0, 1.0]),
    ("accelerate", [0.3, 0.0, 1.0, 1.0]),
    ("emphasized", [0.05, 0.7, 0.1, 1.0]),
    ("ease", [0.25, 0.1, 0.25, 1.0]),
    ("anticipate", [0.68, -0.6, 0.32, 1.6]),
];

/// `MOTION_EASES`.
pub fn motion_eases() -> Vec<&'static str> {
    let mut v = EASE_KEYS.to_vec();
    v.extend(BEZIER.iter().map(|(n, _)| *n));
    v.push("cubic(x1,y1,x2,y2)");
    v
}

fn ease_type(e: &str) -> Option<&'static str> {
    Some(match e {
        "linear" => "LINEAR",
        "in" => "EASE_IN",
        "out" => "EASE_OUT",
        "in-out" => "EASE_IN_AND_OUT",
        "back" => "EASE_OUT_BACK",
        "back-in" => "EASE_IN_BACK",
        "back-in-out" => "EASE_IN_AND_OUT_BACK",
        "spring" | "bounce" => "BOUNCY",
        "gentle" => "GENTLE",
        "quick" => "QUICK",
        "slow" => "SLOW",
        "hold" => "HOLD",
        _ => return None,
    })
}

// bezier([x1, y1, x2, y2]): a missing number is left out (undefined), NaN becomes null
fn bezier(p: &[f64]) -> Value {
    let mut m = Obj::new();
    for (i, k) in ["x1", "y1", "x2", "y2"].iter().enumerate() {
        if let Some(x) = p.get(i) {
            m.insert(k.to_string(), num(*x));
        }
    }
    obj! { "type" => "CUSTOM_CUBIC_BEZIER", "easingFunctionCubicBezier" => Value::Object(m) }
}

/// `toEasing(e)`: a spec ease → a Figma easing (None = undefined).
pub fn to_easing(e: Option<&Value>) -> Result<Option<Value>> {
    let Some(e) = js::nullish(e) else { return Ok(None) };
    if let Value::Array(a) = e {
        if a.len() == 4 {
            return Ok(Some(bezier(&a.iter().map(js::number).collect::<Vec<_>>())));
        }
    }
    let s = js::string(e);
    if let Some(m) = cubic_re().captures(&s) {
        return Ok(Some(bezier(&m[1].split(',').map(js::str_to_number).collect::<Vec<_>>())));
    }
    if let Some(t) = ease_type(&s) {
        return Ok(Some(obj! { "type" => t }));
    }
    if let Some((_, b)) = BEZIER.iter().find(|(n, _)| *n == s) {
        return Ok(Some(bezier(b)));
    }
    Err(anyhow!("Unknown ease \"{}\" — use {}", s, motion_eases().join(", ")))
}

fn hex_rgba(h: Option<&Value>) -> Result<Value> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"^#?([0-9a-fA-F]{6})([0-9a-fA-F]{2})?$").unwrap());
    let s = js::to_string(h);
    let m = re.captures(&s).ok_or_else(|| anyhow!("fill: \"{s}\" is not a hex colour"))?;
    let n = i64::from_str_radix(&m[1], 16).unwrap_or(0);
    Ok(obj! {
        "r" => ((n >> 16) & 255) as f64 / 255.0,
        "g" => ((n >> 8) & 255) as f64 / 255.0,
        "b" => (n & 255) as f64 / 255.0,
        "a" => m.get(2).map_or(1.0, |a| i64::from_str_radix(a.as_str(), 16).unwrap_or(0) as f64 / 255.0),
    })
}

fn rest(prop: &str) -> Option<Value> {
    Some(match prop {
        "opacity" | "scale" | "draw" => Value::from(1),
        "x" | "y" | "rotate" => Value::from(0),
        "move" => serde_json::json!([0, 0]),
        _ => return None,
    })
}

// v[i] for an array value, else the fallback
fn index_or(v: Option<&Value>, i: usize, fallback: Option<Value>) -> Option<Value> {
    match v {
        Some(Value::Array(a)) => a.get(i).cloned(),
        _ => fallback,
    }
}

/// Each spec prop → Figma fields and the value converter (`PROP`).
fn prop_fields(prop: &str, v: Option<&Value>) -> Vec<(&'static str, Option<Value>)> {
    match prop {
        "opacity" => vec![("OPACITY", v.cloned())],
        "move" => vec![("TRANSLATION_X", index_or(v, 0, v.cloned())), ("TRANSLATION_Y", index_or(v, 1, Some(Value::from(0))))],
        "x" => vec![("TRANSLATION_X", v.cloned())],
        "y" => vec![("TRANSLATION_Y", v.cloned())],
        "scale" => vec![("SCALE_X", index_or(v, 0, v.cloned())), ("SCALE_Y", index_or(v, 1, v.cloned()))],
        "rotate" => vec![("ROTATION", Some(num(rotate_to_figma(js::to_number(v)))))],
        "draw" => vec![("PATH_TRIM_END", v.cloned())],
        "width" => vec![("WIDTH", v.cloned())],
        "height" => vec![("HEIGHT", v.cloned())],
        "radius" => vec![("CORNER_RADIUS", v.cloned())],
        _ => vec![],
    }
}

/// `tracksToItems(tracks)`: spec tracks → plugin items `[{ layer, stagger (s), field, keys: [{ t (s), value, easing }] }]`.
/// A keyframe's easing shapes the segment that LEAVES it, so a from/to ease goes on the start key; in keys
/// form an ease on [ms, v, ease] leaves that key.
pub fn tracks_to_items(tracks: &[Value]) -> Result<Vec<Value>> {
    let mut items = vec![];
    for (i, tr) in tracks.iter().enumerate() {
        let trv = Some(tr);
        let prop = match js::get(trv, "prop") {
            Some(Value::String(p)) if MOTION_PROPS.contains(&p.as_str()) => p.clone(),
            other => return Err(anyhow!("track {}: prop \"{}\" — use {}", i + 1, js::to_string(other), MOTION_PROPS.join(", "))),
        };
        // keys: (ms, v, ease) — all as given
        let mut keys: Vec<(Option<Value>, Option<Value>, Option<Value>)> = vec![];
        match js::get(trv, "keys") {
            Some(Value::Array(ks)) if ks.len() >= 2 => {
                for (k, kk) in ks.iter().enumerate() {
                    let at = |j: usize| kk.as_array().and_then(|a| a.get(j)).cloned();
                    let ease = if k == ks.len() - 1 { at(2) } else { js::nullish(at(2).as_ref()).cloned().or_else(|| js::get(trv, "ease").cloned()) };
                    keys.push((at(0), at(1), ease));
                }
            }
            _ => {
                let start = js::nullish(js::get(trv, "start")).cloned().unwrap_or(Value::from(0));
                let end = match js::nullish(js::get(trv, "end")) {
                    Some(e) => e.clone(),
                    None => num(js::number(&start) + js::nullish(js::get(trv, "duration")).map_or(300.0, js::number)),
                };
                let from = js::nullish(js::get(trv, "from")).cloned().or_else(|| rest(&prop));
                let to = js::nullish(js::get(trv, "to")).cloned().or_else(|| rest(&prop));
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(anyhow!("track {}: {} needs from and to", i + 1, prop));
                };
                let ease = js::nullish(js::get(trv, "ease")).cloned().unwrap_or_else(|| "standard".into());
                keys.push((Some(start), Some(from), Some(ease)));
                keys.push((Some(end), Some(to), None));
            }
        }
        let stagger = js::nullish(js::get(trv, "stagger")).map_or(0.0, js::number) / 1000.0;
        let layer = js::nullish(js::get(trv, "layer")).cloned().unwrap_or_else(|| "all".into());
        let t_of = |ms: &Option<Value>| num(js::to_number(ms.as_ref()) / 1000.0);
        if prop == "fill" {
            let mut ks = vec![];
            for (ms, v, ease) in &keys {
                ks.push(obj! { "t" => t_of(ms), "value" => obj! { "type" => "COLOR", "value" => hex_rgba(v.as_ref())? }, "easing" => to_easing(ease.as_ref())? });
            }
            items.push(obj! { "layer" => layer, "stagger" => stagger, "field" => obj! { "type" => "INDEXED_ITEM", "collection" => "fills", "index" => 0i64 }, "keys" => Value::Array(ks) });
            continue;
        }
        let n_fields = prop_fields(&prop, None).len();
        for fi in 0..n_fields {
            // TRANSLATION_X/Y keys are offsets from where the layer sits (its base is its x / y), so move goes in as given
            let name = prop_fields(&prop, None)[fi].0;
            let mut ks = vec![];
            for (ms, v, ease) in &keys {
                let value = prop_fields(&prop, v.as_ref()).swap_remove(fi).1;
                ks.push(obj! { "t" => t_of(ms), "value" => obj! { "type" => "FLOAT", "value" => value }, "easing" => to_easing(ease.as_ref())? });
            }
            items.push(obj! { "layer" => layer.clone(), "stagger" => stagger, "field" => obj! { "type" => "PROPERTY", "name" => name }, "keys" => Value::Array(ks) });
        }
    }
    Ok(items)
}

/// `presetsToItems(presets, available)`: presets `[{ layer, style, props?, delay? ms, duration? ms, stagger? ms }]`.
pub fn presets_to_items(presets: &[Value], available: &[Value]) -> Result<Vec<Value>> {
    let mut out = vec![];
    for (i, pr) in presets.iter().enumerate() {
        let p = Some(pr);
        let want = js::to_string(js::get(p, "style")).to_lowercase();
        let s = available.iter().find(|a| {
            let a = Some(*a);
            js::to_string(js::get(a, "styleId")).to_lowercase() == want || js::to_string(js::get(a, "name")).to_lowercase().ends_with(&format!(".{want}"))
        });
        let Some(s) = s else {
            let ids: Vec<Value> = available.iter().map(|a| js::get(Some(a), "styleId").cloned().unwrap_or(Value::Null)).collect();
            return Err(anyhow!("preset {}: no animation style \"{}\" — this file has {}", i + 1, js::to_string(js::get(p, "style")), js::join(&ids, ", ")));
        };
        let mut props = Obj::new();
        match js::nullish(js::get(p, "props")) {
            Some(Value::Object(o)) => {
                for (k, v) in js::entries(o) {
                    props.insert(k.clone(), v.clone());
                }
            }
            Some(Value::Array(a)) => {
                for (k, v) in a.iter().enumerate() {
                    props.insert(k.to_string(), v.clone());
                }
            }
            _ => {}
        }
        let delay = js::nullish(js::get(p, "delay")).map(|d| js::number(d) / 1000.0);
        let duration = js::nullish(js::get(p, "duration")).map(|d| js::number(d) / 1000.0);
        if let Some(d) = delay {
            props.insert("delay".into(), num(d));
        }
        if let Some(d) = duration {
            props.insert("duration".into(), num(d));
        }
        let mut config = Obj::new();
        js::put(&mut config, "duration", duration);
        config.insert("props".into(), Value::Object(props));
        out.push(obj! {
            "layer" => js::nullish(js::get(p, "layer")).cloned().unwrap_or_else(|| "all".into()),
            "stagger" => js::nullish(js::get(p, "stagger")).map_or(0.0, js::number) / 1000.0,
            "kind" => "style",
            "styleId" => js::get(Some(s), "styleId").cloned(),
            "config" => Value::Object(config),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- Figma Motion → create_lottie spec
fn from_field(field: &str) -> Option<(&'static str, Option<usize>)> {
    Some(match field {
        "OPACITY" => ("opacity", None),
        "ROTATION" => ("rotate", None),
        "PATH_TRIM_END" => ("draw", None),
        "TRANSLATION_X" => ("move", Some(0)),
        "TRANSLATION_Y" => ("move", Some(1)),
        "TRANSLATION_XY" => ("move", None),
        "SCALE_X" => ("scale", Some(0)),
        "SCALE_Y" => ("scale", Some(1)),
        "SCALE_XY" => ("scale", None),
        "fills[0]" => ("fill", None),
        "strokes[0]" => ("stroke", None),
        "WIDTH" => ("width", None),
        "HEIGHT" => ("height", None),
        "CORNER_RADIUS" => ("radius", None),
        _ => return None,
    })
}

fn from_ease(e: &str) -> Option<&'static str> {
    Some(match e {
        "LINEAR" => "linear",
        "EASE_IN" => "in",
        "EASE_OUT" => "out",
        "EASE_IN_AND_OUT" => "in-out",
        "EASE_OUT_BACK" => "back",
        "EASE_IN_BACK" => "back-in",
        "EASE_IN_AND_OUT_BACK" => "anticipate",
        "HOLD" => "hold",
        "BOUNCY" => "spring",
        _ => return None,
    })
}
// Figma springs without overshoot ≈ a curve
fn near_ease(e: &str) -> Option<&'static str> {
    Some(match e {
        "GENTLE" | "SLOW" => "decelerate",
        "QUICK" => "standard",
        _ => return None,
    })
}

fn ease_from(e: Option<&Value>, warn: &mut dyn FnMut(String)) -> Option<String> {
    let e = js::nullish(e)?;
    if let Value::String(s) = e {
        if let Some(inner) = s.strip_prefix("cubic(") {
            // one of our named curves written as a custom bezier → its name again (so tokens get real names)
            let mut chars: Vec<char> = inner.chars().collect();
            chars.pop();
            let body: String = chars.into_iter().collect();
            let p: Vec<f64> = body.split(',').map(js::str_to_number).collect();
            let named = BEZIER.iter().find(|(_, b)| b.iter().enumerate().all(|(i, x)| p.get(i).is_some_and(|y| (x - y).abs() < 0.005)));
            return Some(named.map_or(s.clone(), |(n, _)| n.to_string()));
        }
        if let Some(n) = from_ease(s) {
            return Some(n.into());
        }
        if let Some(n) = near_ease(s) {
            warn(format!("{s} spring → \"{n}\" curve (Lottie has no springs)"));
            return Some(n.into());
        }
    }
    let field = |k: &str| match e {
        Value::Object(o) => js::truthy(o.get(k)),
        _ => false,
    };
    if field("spring") {
        warn("custom spring → \"spring\" (approximated)".into());
        return Some("spring".into());
    }
    if field("variable") {
        warn("easing bound to a variable → \"standard\"".into());
        return Some("standard".into());
    }
    warn(format!("unknown easing {} → \"standard\"", js::stringify(e)));
    Some("standard".into())
}

// Apply the keyframe operation: SET is the value, OFFSET adds to the base, SCALE multiplies it.
fn op_value(op: Option<&Value>, base: Option<&Value>, v: Option<&Value>) -> Option<Value> {
    let op = op.and_then(|o| o.as_str());
    let combine = |f: fn(f64, f64) -> f64, unit: f64| -> Option<Value> {
        match v {
            Some(Value::Array(a)) => Some(Value::Array(
                a.iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let b = js::nullish(match base {
                            Some(Value::Array(b)) => b.get(i),
                            Some(Value::Object(o)) => o.get(&i.to_string()),
                            _ => None,
                        })
                        .map_or(unit, js::number);
                        num(f(b, js::number(x)))
                    })
                    .collect(),
            )),
            _ => Some(num(f(js::nullish(base).map_or(unit, js::number), js::to_number(v)))),
        }
    };
    match op {
        Some("OFFSET") => combine(|b, x| b + x, 0.0),
        Some("SCALE") => combine(|b, x| b * x, 1.0),
        _ => v.cloned(),
    }
}

/// A key in spec form: `[ms, value]` or `[ms, value, ease]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecKey {
    pub ms: f64,
    pub v: Option<Value>,
    pub ease: Option<String>,
}

impl SpecKey {
    pub fn to_json(&self) -> Value {
        let mut a = vec![num(self.ms), self.v.clone().unwrap_or(Value::Null)];
        if let Some(e) = &self.ease {
            a.push(Value::String(e.clone()));
        }
        Value::Array(a)
    }
}

/// One prop's keys in spec form.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecTrack {
    pub prop: String,
    pub keys: Vec<SpecKey>,
}

impl SpecTrack {
    pub fn to_json(&self) -> Value {
        obj! { "prop" => self.prop.as_str(), "keys" => Value::Array(self.keys.iter().map(|k| k.to_json()).collect()) }
    }
}

// Value of a key list at time t (linear between keys) — only to fill an axis that has no key there.
fn value_at(keys: &[SpecKey], t: f64) -> Option<Value> {
    if keys.is_empty() {
        return None;
    }
    if t <= keys[0].ms {
        return keys[0].v.clone();
    }
    for i in 1..keys.len() {
        if t <= keys[i].ms {
            let (a, b) = (&keys[i - 1], &keys[i]);
            let (av, bv) = (js::to_number(a.v.as_ref()), js::to_number(b.v.as_ref()));
            return Some(num(av + ((bv - av) * (t - a.ms)) / (b.ms - a.ms)));
        }
    }
    keys.last().unwrap().v.clone()
}

/// `nodeTracks(n, warnings)`: one layer's Motion tracks in spec form (X/Y axes merged).
pub fn node_tracks(n: &Value, warnings: &mut Vec<String>) -> Result<Vec<SpecTrack>> {
    let nv = Some(n);
    let wh = format!("{} ({})", js::to_string(js::get(nv, "name")), js::to_string(js::get(nv, "id")));
    let mut out = vec![];
    let mut axes: Vec<(&'static str, [Option<Vec<SpecKey>>; 2])> = vec![]; // move / scale split by axis, merged below
    for f in arr(js::get(nv, "tracks"), "length")? {
        let fv = Some(f);
        let field = js::to_string(js::get(fv, "field"));
        let Some((prop, axis)) = from_field(&field) else {
            warnings.push(format!("{wh}: {field} is not supported for export yet; skipped"));
            continue;
        };
        let ftracks = arr(js::get(fv, "tracks"), "length")?;
        if ftracks.len() > 1 {
            warnings.push(format!("{wh}: {field} has {} stacked tracks — used the first", ftracks.len()));
        }
        let t = ftracks.first();
        let base = js::get(fv, "base");
        let op = js::get(t, "op");
        let mut keys = vec![];
        for k in arr(js::get(t, "keys"), "map")? {
            let kv = Some(k);
            let raw = if prop == "move" { js::get(kv, "v").cloned() } else { op_value(op, base, js::get(kv, "v")) };
            // translation keys are already offsets from the layer's position (base = its x / y), so they read as move as-is
            let v = if prop == "rotate" { Some(num(js::round(((js::to_number(raw.as_ref()) * 180.0) / PI) * 1000.0) / 1000.0)) } else { raw };
            let mut ws = vec![];
            let ease = ease_from(js::get(kv, "ease"), &mut |w| ws.push(w));
            warnings.extend(ws.into_iter().map(|w| format!("{wh} {field}: {w}")));
            keys.push(SpecKey { ms: ms(js::get(kv, "t")), v, ease: ease.filter(|e| !e.is_empty()) });
        }
        match axis {
            None => out.push(SpecTrack { prop: prop.into(), keys }),
            Some(ax) => {
                let i = match axes.iter().position(|(p, _)| *p == prop) {
                    Some(i) => i,
                    None => {
                        axes.push((prop, [None, None]));
                        axes.len() - 1
                    }
                };
                axes[i].1[ax] = Some(keys);
            }
        }
    }
    for (prop, [kx, ky]) in axes {
        let mut times: Vec<f64> = vec![];
        for k in kx.iter().flatten().chain(ky.iter().flatten()) {
            if !times.iter().any(|t| *t == k.ms || (t.is_nan() && k.ms.is_nan())) {
                times.push(k.ms);
            }
        }
        times.sort_by(|a, b| js::cmp_num(a - b));
        if let (Some(x), Some(y)) = (&kx, &ky) {
            if x.len() != y.len() || x.iter().enumerate().any(|(i, k)| y.get(i).is_none_or(|yk| k.ms != yk.ms)) {
                warnings.push(format!("{wh}: {prop} X and Y keys are at different times — merged with linear in-betweens"));
            }
        }
        let rest_axis = if prop == "move" { 0 } else { 1 };
        let at = |keys: &Option<Vec<SpecKey>>, t: f64| match keys {
            Some(k) => value_at(k, t),
            None => Some(Value::from(rest_axis)),
        };
        let keys = times
            .iter()
            .map(|&t| {
                let ease = kx.as_ref().and_then(|k| k.iter().find(|k| k.ms == t)).or_else(|| ky.as_ref().and_then(|k| k.iter().find(|k| k.ms == t))).and_then(|k| k.ease.clone());
                let v = Value::Array(vec![at(&kx, t).unwrap_or(Value::Null), at(&ky, t).unwrap_or(Value::Null)]);
                SpecKey { ms: t, v: Some(v), ease }
            })
            .collect();
        out.push(SpecTrack { prop: prop.into(), keys });
    }
    Ok(out)
}

const LOTTIE_PROPS: [&str; 7] = ["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"];

/// `motionToSpec(r)`: motion_read result → `{ duration ms, tracks (create_lottie spec), warnings, split? }`.
/// Lottie animates the frame's top-level layers (the SVG export's groups), so only those are carried over.
pub fn motion_to_spec(r: &Value) -> Result<Value> {
    let rv = Some(r);
    let mut warnings = vec![];
    let mut split: Vec<Value> = vec![];
    let tl0 = js::get(rv, "timelines").and_then(|t| t.as_array()).and_then(|a| a.first());
    let duration = ms(Some(js::nullish(js::get(tl0, "duration")).unwrap_or(&Value::from(1))));
    let root_id = js::get(js::get(rv, "root"), "id");
    // `for (const n of r.animated)`: V8 names the expression when it is not iterable
    let Some(Value::Array(animated)) = js::get(rv, "animated") else {
        return Err(anyhow!("r.animated is not iterable"));
    };
    let mut tracks = vec![];
    for n in animated {
        let nv = Some(n);
        let wh = format!("{} ({})", js::to_string(js::get(nv, "name")), js::to_string(js::get(nv, "id")));
        if arr(js::get(nv, "tracks"), "length")?.is_empty() {
            continue;
        }
        if js::strict_eq(js::get(nv, "id"), root_id) {
            warnings.push(format!("{wh}: the frame itself is animated — Lottie only moves its layers; skipped"));
            continue;
        }
        if !js::strict_eq(js::get(nv, "parentId"), root_id) {
            // nested: open the groups above it in the SVG (needs the plugin's path; older plugins don't send it)
            let Some(Value::Array(path)) = js::get(nv, "path") else {
                warnings.push(format!("{wh}: nested layer — reopen the Figma plugin so wara can animate layers inside groups; skipped"));
                continue;
            };
            for g in path {
                if !split.iter().any(|s| js::strict_eq(Some(s), Some(g))) {
                    split.push(g.clone());
                }
            }
        }
        let nth = js::get(nv, "nth");
        let name = js::get(nv, "name");
        let layer: Option<Value> = if js::to_number(nth) > 1.0 { Some(Value::String(format!("{}_{}", js::to_string(name), js::to_string(nth)))) } else { name.cloned() };
        for t in node_tracks(n, &mut warnings)? {
            if LOTTIE_PROPS.contains(&t.prop.as_str()) {
                let mut o = Obj::new();
                js::put(&mut o, "layer", layer.clone());
                o.insert("prop".into(), Value::String(t.prop.clone()));
                o.insert("keys".into(), Value::Array(t.keys.iter().map(|k| k.to_json()).collect()));
                tracks.push(Value::Object(o));
            } else {
                warnings.push(format!("{wh}: {} has no Lottie equivalent; skipped", t.prop));
            }
        }
    }
    // a group that animates itself and is also opened for its children loses its own motion in Lottie
    for n in animated {
        let nv = Some(n);
        let has_tracks = js::get(nv, "tracks").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty());
        if has_tracks && split.iter().any(|s| js::strict_eq(Some(s), js::get(nv, "name"))) {
            warnings.push(format!(
                "{} ({}): animates and has animated layers inside — Lottie can't nest them, its own motion is dropped (animate the children, or wrap it)",
                js::to_string(js::get(nv, "name")),
                js::to_string(js::get(nv, "id"))
            ));
        }
    }
    let mut o = obj! { "duration" => duration, "tracks" => Value::Array(tracks), "warnings" => Value::Array(warnings.into_iter().map(Value::String).collect()) };
    if !split.is_empty() {
        o.as_object_mut().unwrap().insert("split".into(), Value::Array(split));
    }
    Ok(o)
}
