//! Port of `legacy-node/lottie.mjs`: SVG art + a short motion spec → a Lottie (bodymovin) JSON file.
//! The AI sends what moves and how ("check: draw 0→1, 0–400ms, out"); this file writes the keyframes,
//! so a few hundred tokens of spec stand in for thousands of tokens of Lottie JSON.
//!
//! Layers = the top-level elements of the SVG (a lone wrapping <g> is unwrapped). Names come from
//! id / data-name (Figma exports layer names as ids), else "<tag>-<n>". Transforms are baked into the
//! points; every layer pivots around its own centre, so scale and rotate look right without setup.

use crate::js::{self, num, nums, Obj};
use crate::obj;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::sync::OnceLock;

macro_rules! re {
    ($s:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new(&$s).unwrap())
    }};
}

// fdlibm, as V8's Math functions use
fn sin(x: f64) -> f64 {
    libm::sin(x)
}
fn cos(x: f64) -> f64 {
    libm::cos(x)
}
fn tan(x: f64) -> f64 {
    libm::tan(x)
}
fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
fn exp(x: f64) -> f64 {
    libm::exp(x)
}

// ---------------------------------------------------------------- SVG → tree
#[derive(Debug, Clone)]
pub struct SvgNode {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub children: Vec<SvgNode>,
}

impl SvgNode {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(|s| s.as_str())
    }
}

fn unescape(s: &str) -> Result<String> {
    let r = re!(r"&(#(?i:x?[0-9a-f]+)|[A-Za-z0-9_]+);");
    let mut bad: Option<f64> = None;
    let out = r.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        if let Some(rest) = e.strip_prefix('#') {
            let code = if let Some(hex) = rest.strip_prefix('x') { js::parse_int(hex, 16) } else { js::str_to_number(rest) };
            // String.fromCodePoint (a lone surrogate can't live in a Rust string: U+FFFD instead)
            if code.is_finite() && (0.0..=1_114_111.0).contains(&code) && code.fract() == 0.0 {
                return char::from_u32(code as u32).unwrap_or('\u{fffd}').to_string();
            }
            bad.get_or_insert(code);
            return String::new();
        }
        match e {
            "amp" => "&".into(),
            "lt" => "<".into(),
            "gt" => ">".into(),
            "quot" => "\"".into(),
            "apos" => "'".into(),
            _ => c[0].to_string(),
        }
    });
    if let Some(code) = bad {
        return Err(anyhow!("Invalid code point {}", js::num_str(code)));
    }
    Ok(out.into_owned())
}

/// `parseSvg(text)`: the <svg> element as a tree.
pub fn parse_svg(text: &str) -> Result<SvgNode> {
    let ws = js::WS;
    let name = "[A-Za-z0-9_:.-]";
    let tag_re = re!(format!(
        r#"<!--[\s\S]*?-->|<\?[\s\S]*?\?>|<!\[(?i:cdata)\[[\s\S]*?\]\]>|<!(?i:doctype)[^>]*>|<(/?)({name}+)((?:[{ws}]+{name}+[{ws}]*=[{ws}]*(?:"[^"]*"|'[^']*'))*)[{ws}]*(/?)>"#
    ));
    let attr_re = re!(format!(r#"({name}+)[{ws}]*=[{ws}]*(?:"([^"]*)"|'([^']*)')"#, name = "[A-Za-z0-9_:.-]", ws = js::WS));
    // stack of open nodes; children are attached when a node closes (or at the end)
    let mut stack: Vec<SvgNode> = vec![SvgNode { tag: "#root".into(), attrs: HashMap::new(), children: vec![] }];
    for m in tag_re.captures_iter(text) {
        let Some(tag) = m.get(2) else { continue };
        let close = !m.get(1).map_or("", |x| x.as_str()).is_empty();
        if close {
            if stack.len() > 1 {
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(n);
            }
            continue;
        }
        let mut attrs = HashMap::new();
        for a in attr_re.captures_iter(m.get(3).map_or("", |x| x.as_str())) {
            let v = a.get(2).or(a.get(3)).map_or("", |x| x.as_str());
            attrs.insert(a[1].to_string(), unescape(v)?);
        }
        let node = SvgNode { tag: tag.as_str().to_lowercase(), attrs, children: vec![] };
        if m.get(4).map_or("", |x| x.as_str()).is_empty() {
            stack.push(node);
        } else {
            stack.last_mut().unwrap().children.push(node);
        }
    }
    while stack.len() > 1 {
        let n = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(n);
    }
    let root = stack.pop().unwrap();
    root.children.into_iter().find(|c| c.tag == "svg").ok_or_else(|| anyhow!("Not an SVG: no <svg> element"))
}

// ---------------------------------------------------------------- transforms
type M = [f64; 6];
type Pt = [f64; 2];
const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
fn mul(m: &M, n: &M) -> M {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}
fn apply(m: &M, p: &Pt) -> Pt {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

fn split_ws_comma(s: &str) -> Vec<&str> {
    // s.split(/[\s,]+/)
    let r = re!(format!(r"[{},]+", js::WS));
    r.split(s).collect()
}

fn parse_transform(s: Option<&str>) -> M {
    let mut m = IDENTITY;
    let r = re!(r"([A-Za-z0-9_]+)[\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]*\(([^)]*)\)");
    for c in r.captures_iter(s.unwrap_or("")) {
        let f = &c[1];
        let a: Vec<f64> = split_ws_comma(&c[2]).into_iter().filter(|x| !x.is_empty()).map(js::str_to_number).collect();
        let at = |i: usize| a.get(i).copied();
        let mut t = IDENTITY;
        if f == "matrix" && a.len() == 6 {
            t = [a[0], a[1], a[2], a[3], a[4], a[5]];
        } else if f == "translate" {
            t = [1.0, 0.0, 0.0, 1.0, at(0).unwrap_or(0.0), at(1).unwrap_or(0.0)];
        } else if f == "scale" {
            let x = at(0).unwrap_or(f64::NAN);
            t = [x, 0.0, 0.0, at(1).unwrap_or(x), 0.0, 0.0];
        } else if f == "rotate" {
            let r = (at(0).unwrap_or(0.0) * PI) / 180.0;
            let (c, sn) = (cos(r), sin(r));
            t = [c, sn, -sn, c, 0.0, 0.0];
            if a.len() == 3 {
                t = mul(&mul(&[1.0, 0.0, 0.0, 1.0, a[1], a[2]], &t), &[1.0, 0.0, 0.0, 1.0, -a[1], -a[2]]);
            }
        } else if f == "skewX" {
            t = [1.0, 0.0, tan((at(0).unwrap_or(f64::NAN) * PI) / 180.0), 1.0, 0.0, 0.0];
        } else if f == "skewY" {
            t = [1.0, tan((at(0).unwrap_or(f64::NAN) * PI) / 180.0), 0.0, 1.0, 0.0, 0.0];
        }
        m = mul(&m, &t);
    }
    m
}

// ---------------------------------------------------------------- path data → cubic subpaths
/// Subpath: `segs[i]` joins `pts[i]` → `pts[i+1]` (the closing segment last).
#[derive(Debug, Clone, PartialEq)]
pub struct Sub {
    pub closed: bool,
    pub pts: Vec<Pt>,
    pub segs: Vec<[Pt; 2]>,
}

impl Sub {
    pub fn to_json(&self) -> Value {
        obj! {
            "closed" => self.closed,
            "pts" => Value::Array(self.pts.iter().map(|p| nums(p)).collect()),
            "segs" => Value::Array(self.segs.iter().map(|s| Value::Array(vec![nums(&s[0]), nums(&s[1])])).collect()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(x1: f64, y1: f64, rx: f64, ry: f64, phi: f64, large: f64, sweep: f64, x2: f64, y2: f64) -> Vec<[Pt; 3]> {
    let falsy = |x: f64| x == 0.0 || x.is_nan();
    if falsy(rx) || falsy(ry) {
        return vec![[[x1, y1], [x2, y2], [x2, y2]]];
    }
    let rad = (phi * PI) / 180.0;
    let (cs, sn) = (cos(rad), sin(rad));
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = cs * dx + sn * dy;
    let y1p = -sn * dx + cs * dy;
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let numr = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let co = sign * js::max(0.0, numr / (rx * rx * y1p * y1p + ry * ry * x1p * x1p)).sqrt();
    let cxp = (co * rx * y1p) / ry;
    let cyp = (-co * ry * x1p) / rx;
    let cx = cs * cxp - sn * cyp + (x1 + x2) / 2.0;
    let cy = sn * cxp + cs * cyp + (y1 + y2) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let t1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dt = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if falsy(sweep) && dt > 0.0 {
        dt -= 2.0 * PI;
    }
    if !falsy(sweep) && dt < 0.0 {
        dt += 2.0 * PI;
    }
    let n0 = (dt.abs() / (PI / 2.0) - 1e-9).ceil();
    let n = if falsy(n0) { 1.0 } else { n0 };
    let step = dt / n;
    let k = (4.0 / 3.0) * tan(step / 4.0);
    let pt = |t: f64| [cx + rx * cos(t) * cs - ry * sin(t) * sn, cy + rx * cos(t) * sn + ry * sin(t) * cs];
    let der = |t: f64| [-rx * sin(t) * cs - ry * cos(t) * sn, -rx * sin(t) * sn + ry * cos(t) * cs];
    let mut out = vec![];
    let mut i = 0.0;
    while i < n {
        let a = t1 + i * step;
        let b = a + step;
        let (p0, p1, d0, d1) = (pt(a), pt(b), der(a), der(b));
        out.push([[p0[0] + k * d0[0], p0[1] + k * d0[1]], [p1[0] - k * d1[0], p1[1] - k * d1[1]], p1]);
        i += 1.0;
    }
    out
}

/// `parsePath(d)`.
pub fn parse_path(d: &str) -> Vec<Sub> {
    let tok_re = re!(r"[a-df-zA-DF-Z]|[-+]?(?:[0-9]*\.[0-9]+|[0-9]+\.?)(?:[eE][-+]?[0-9]+)?");
    let toks: Vec<&str> = tok_re.find_iter(d).map(|m| m.as_str()).collect();
    let mut subs: Vec<Sub> = vec![];
    let mut sub: Option<usize> = None; // index into subs
    let mut i = 0usize;
    let mut cmd = String::new();
    let mut cur: Pt = [0.0, 0.0];
    let mut start: Pt = [0.0, 0.0];
    let mut last_ctrl: Option<Pt> = None;
    let mut last_cmd = String::new();
    let has_letter = |t: &str| t.bytes().any(|b| b.is_ascii_alphabetic());
    let is_cs = |t: &str| t.bytes().any(|b| matches!(b, b'C' | b'S' | b'c' | b's'));
    let is_qt = |t: &str| t.bytes().any(|b| matches!(b, b'Q' | b'T' | b'q' | b't'));
    macro_rules! num {
        () => {{
            let v = toks.get(i).map_or(f64::NAN, |t| js::str_to_number(t));
            i += 1;
            v
        }};
    }
    // begin a subpath at p
    macro_rules! begin {
        ($p:expr) => {{
            subs.push(Sub { closed: false, pts: vec![$p], segs: vec![] });
            sub = Some(subs.len() - 1);
        }};
    }
    macro_rules! cubic {
        ($c1:expr, $c2:expr, $p:expr) => {{
            let (c1, c2, p) = ($c1, $c2, $p);
            if sub.is_none() {
                begin!(cur);
            }
            let s = &mut subs[sub.unwrap()];
            s.segs.push([c1, c2]);
            s.pts.push(p);
            cur = p;
        }};
    }
    while i < toks.len() {
        if has_letter(toks[i]) {
            cmd = toks[i].to_string();
            i += 1;
        } else if cmd.is_empty() {
            i += 1;
            continue;
        }
        let rel = cmd == cmd.to_lowercase() && cmd != "z" && cmd != "Z";
        let c = cmd.to_uppercase();
        macro_rules! p {
            ($x:expr, $y:expr) => {{
                let (x, y) = ($x, $y);
                if rel {
                    [cur[0] + x, cur[1] + y]
                } else {
                    [x, y]
                }
            }};
        }
        match c.as_str() {
            "M" => {
                let x = num!();
                let y = num!();
                let p = p!(x, y);
                begin!(p);
                cur = p;
                start = p;
                cmd = if rel { "l".into() } else { "L".into() };
            }
            "L" => {
                let x = num!();
                let y = num!();
                let p = p!(x, y);
                cubic!(cur, p, p);
            }
            "H" => {
                let x = num!();
                let p = [if rel { cur[0] + x } else { x }, cur[1]];
                cubic!(cur, p, p);
            }
            "V" => {
                let y = num!();
                let p = [cur[0], if rel { cur[1] + y } else { y }];
                cubic!(cur, p, p);
            }
            "C" => {
                let (a, b) = (num!(), num!());
                let c1 = p!(a, b);
                let (a, b) = (num!(), num!());
                let c2 = p!(a, b);
                let (a, b) = (num!(), num!());
                let p = p!(a, b);
                cubic!(c1, c2, p);
                last_ctrl = Some(c2);
            }
            "S" => {
                let c1 = match last_ctrl {
                    Some(lc) if is_cs(&last_cmd) => [2.0 * cur[0] - lc[0], 2.0 * cur[1] - lc[1]],
                    _ => cur,
                };
                let (a, b) = (num!(), num!());
                let c2 = p!(a, b);
                let (a, b) = (num!(), num!());
                let p = p!(a, b);
                cubic!(c1, c2, p);
                last_ctrl = Some(c2);
            }
            "Q" | "T" => {
                let q = if c == "Q" {
                    let (a, b) = (num!(), num!());
                    p!(a, b)
                } else {
                    match last_ctrl {
                        Some(lc) if is_qt(&last_cmd) => [2.0 * cur[0] - lc[0], 2.0 * cur[1] - lc[1]],
                        _ => cur,
                    }
                };
                let (a, b) = (num!(), num!());
                let p = p!(a, b);
                let c1 = [cur[0] + (2.0 / 3.0) * (q[0] - cur[0]), cur[1] + (2.0 / 3.0) * (q[1] - cur[1])];
                let c2 = [p[0] + (2.0 / 3.0) * (q[0] - p[0]), p[1] + (2.0 / 3.0) * (q[1] - p[1])];
                cubic!(c1, c2, p);
                last_ctrl = Some(q);
            }
            "A" => {
                let (rx, ry, phi, large, sweep) = (num!(), num!(), num!(), num!(), num!());
                let (a, b) = (num!(), num!());
                let p = p!(a, b);
                for [c1, c2, q] in arc_to_cubics(cur[0], cur[1], rx, ry, phi, large, sweep, p[0], p[1]) {
                    cubic!(c1, c2, q);
                }
            }
            "Z" => {
                if let Some(si) = sub {
                    let s = &mut subs[si];
                    let first = s.pts[0];
                    let last = *s.pts.last().unwrap();
                    // A closing point equal to the first one is dropped: the closing segment takes its curve.
                    if s.pts.len() > 1 && (first[0] - last[0]).hypot(first[1] - last[1]) < 1e-6 {
                        s.pts.pop();
                    } else {
                        s.segs.push([last, first]);
                    }
                    s.closed = true;
                }
                cur = start;
                sub = None;
            }
            _ => {
                i += 1;
            }
        }
        if !is_cs(&c) && !is_qt(&c) {
            last_ctrl = None;
        }
        last_cmd = c;
    }
    subs.into_iter().filter(|s| s.pts.len() > 1 || s.closed).collect()
}

// Basic shapes as path data, so everything goes through one converter.
fn shape_to_path(n: &SvgNode) -> Option<String> {
    let f = |k: &str| {
        let v = js::parse_float(n.attr(k).unwrap_or("0"));
        if v == 0.0 || v.is_nan() {
            0.0
        } else {
            v
        }
    };
    let s = js::num_str;
    match n.tag.as_str() {
        "path" => n.attr("d").map(|d| d.to_string()),
        "rect" => {
            let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
            let mut rx = if n.attr("rx").is_some() { f("rx") } else { f("ry") };
            let mut ry = if n.attr("ry").is_some() { f("ry") } else { rx };
            rx = js::min(rx, w / 2.0);
            ry = js::min(ry, h / 2.0);
            let falsy = |v: f64| v == 0.0 || v.is_nan();
            if falsy(rx) && falsy(ry) {
                return Some(format!("M{} {}H{}V{}H{}Z", s(x), s(y), s(x + w), s(y + h), s(x)));
            }
            let (rxs, rys) = (s(rx), s(ry));
            Some(format!(
                "M{} {}H{}A{rxs} {rys} 0 0 1 {} {}V{}A{rxs} {rys} 0 0 1 {} {}H{}A{rxs} {rys} 0 0 1 {} {}V{}A{rxs} {rys} 0 0 1 {} {}Z",
                s(x + rx),
                s(y),
                s(x + w - rx),
                s(x + w),
                s(y + ry),
                s(y + h - ry),
                s(x + w - rx),
                s(y + h),
                s(x + rx),
                s(x),
                s(y + h - ry),
                s(y + ry),
                s(x + rx),
                s(y),
            ))
        }
        "circle" | "ellipse" => {
            let circle = n.tag == "circle";
            let (cx, cy) = (f("cx"), f("cy"));
            let rx = if circle { f("r") } else { f("rx") };
            let ry = if circle { f("r") } else { f("ry") };
            let (rxs, rys) = (s(rx), s(ry));
            Some(format!("M{} {}A{rxs} {rys} 0 1 0 {} {}A{rxs} {rys} 0 1 0 {} {}Z", s(cx - rx), s(cy), s(cx + rx), s(cy), s(cx - rx), s(cy)))
        }
        "line" => Some(format!("M{} {}L{} {}", s(f("x1")), s(f("y1")), s(f("x2")), s(f("y2")))),
        "polyline" | "polygon" => {
            let p: Vec<f64> = split_ws_comma(js::trim(n.attr("points").unwrap_or(""))).into_iter().map(js::str_to_number).collect();
            if p.len() < 4 {
                return None;
            }
            let count = (p.len() as f64 / 2.0 - 1.0).floor().max(0.0) as usize;
            let mut d = format!("M{} {}", s(p[0]), s(p[1]));
            for k in 0..count {
                d.push_str(&format!("L{} {}", s(p[2 * k + 2]), s(p[2 * k + 3])));
            }
            if n.tag == "polygon" {
                d.push('Z');
            }
            Some(d)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- paint
/// What `parseColor` returns: null, "none", an [r, g, b, a] array (0–1) or `{ ref }` for url(#id).
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    None,
    Rgba([f64; 4]),
    Ref(Option<String>),
}

impl Color {
    pub fn to_json(c: &Option<Color>) -> Value {
        match c {
            None => Value::Null,
            Some(Color::None) => "none".into(),
            Some(Color::Rgba(v)) => nums(v),
            Some(Color::Ref(r)) => obj! { "ref" => r.clone() },
        }
    }
}

/// `parseColor(v)` (None = null / undefined).
pub fn parse_color(v: Option<&str>) -> Option<Color> {
    let v0 = v?;
    let lower = js::trim(v0).to_lowercase();
    let v = match lower.as_str() {
        "black" => "#000000",
        "white" => "#ffffff",
        "red" => "#ff0000",
        "green" => "#008000",
        "blue" => "#0000ff",
        "transparent" => "none",
        "currentcolor" => "#000000",
        other => other,
    };
    if v == "none" || v.is_empty() {
        return Some(Color::None);
    }
    if let Some(m) = re!(r"^#([0-9a-f]{3,8})$").captures(v) {
        let mut h = m[1].to_string();
        if h.len() <= 4 {
            h = h.chars().flat_map(|c| [c, c]).collect();
        }
        let sl = |a: usize, b: usize| -> f64 {
            let b = b.min(h.len());
            if a >= b {
                f64::NAN
            } else {
                js::parse_int(&h[a..b], 16)
            }
        };
        return Some(Color::Rgba([sl(0, 2) / 255.0, sl(2, 4) / 255.0, sl(4, 6) / 255.0, if h.len() == 8 { sl(6, 8) / 255.0 } else { 1.0 }]));
    }
    if let Some(m) = re!(r"^rgba?\(([^)]+)\)$").captures(v) {
        let r = re!(format!(r"[{},/]+", js::WS));
        let p: Vec<f64> = r
            .split(&m[1])
            .filter(|x| !x.is_empty())
            .map(|x| if x.ends_with('%') { js::parse_float(x) * 2.55 } else { js::parse_float(x) })
            .collect();
        let at = |i: usize| p.get(i).copied().unwrap_or(f64::NAN);
        return Some(Color::Rgba([at(0) / 255.0, at(1) / 255.0, at(2) / 255.0, p.get(3).copied().unwrap_or(1.0)]));
    }
    if v.starts_with("url(") {
        return Some(Color::Ref(re!(r#"#([^)'"]+)"#).captures(v).map(|c| c[1].to_string())));
    }
    None
}

fn round(n: f64) -> f64 {
    js::r3(n)
}

// Inherited presentation attributes (style="" wins over attributes).
const PAINT_KEYS: [&str; 11] = ["fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity", "opacity", "fill-rule", "display", "visibility"];

#[derive(Debug, Clone, Default)]
struct Style {
    props: HashMap<String, String>,
    opacity: Option<f64>, // __opacity
}

impl Style {
    fn get(&self, k: &str) -> Option<&str> {
        self.props.get(k).map(|s| s.as_str())
    }
    fn merged(&self, own: &HashMap<String, String>) -> Style {
        let mut s = self.clone();
        for (k, v) in own {
            s.props.insert(k.clone(), v.clone());
        }
        s
    }
}

fn split_style_part(part: &str) -> (Option<String>, Option<String>) {
    let mut it = part.split(':');
    let k = it.next().map(|x| js::trim(x).to_string());
    let v = it.next().map(|x| js::trim(x).to_string());
    (k, v)
}

fn own_style(n: &SvgNode) -> HashMap<String, String> {
    let mut s = HashMap::new();
    for k in PAINT_KEYS {
        if let Some(v) = n.attr(k) {
            s.insert(k.to_string(), v.to_string());
        }
    }
    for part in n.attr("style").unwrap_or("").split(';') {
        if let (Some(k), Some(v)) = split_style_part(part) {
            if !k.is_empty() && !v.is_empty() && PAINT_KEYS.contains(&k.as_str()) {
                s.insert(k, v);
            }
        }
    }
    s
}

// ---------------------------------------------------------------- SVG → layers of drawable items
const DRAWABLE: [&str; 7] = ["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"];
/// `n.attrs[k]` is truthy (present and not "").
fn has(n: &SvgNode, k: &str) -> bool {
    n.attr(k).is_some_and(|v| !v.is_empty())
}

fn is_skip(tag: &str) -> bool {
    matches!(tag, "defs" | "clippath" | "mask" | "lineargradient" | "radialgradient" | "pattern" | "filter" | "style" | "title" | "desc" | "metadata" | "symbol" | "marker")
}
fn is_drawable(tag: &str) -> bool {
    DRAWABLE.contains(&tag)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub radial: bool,
    pub s: Pt,
    pub e: Pt,
    pub stops: Vec<[f64; 5]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid([f64; 4]),
    Gradient { gradient: Gradient, alpha: f64 },
}

impl Paint {
    fn to_json(p: &Option<Paint>) -> Value {
        match p {
            None => Value::Null,
            Some(Paint::Solid(c)) => nums(c),
            Some(Paint::Gradient { gradient: g, alpha }) => obj! {
                "gradient" => obj! {
                    "type" => if g.radial { "radial" } else { "linear" },
                    "s" => nums(&g.s),
                    "e" => nums(&g.e),
                    "stops" => Value::Array(g.stops.iter().map(|s| nums(s)).collect()),
                },
                "alpha" => *alpha,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub subs: Vec<Sub>,
    pub fill: Option<Paint>,
    pub stroke: Option<Paint>,
    pub stroke_width: f64,
    pub cap: String,
    pub join: String,
    pub evenodd: bool,
    pub opacity: f64,
}

#[derive(Debug, Clone)]
pub struct ArtLayer {
    pub name: String,
    pub items: Vec<Item>,
}

/// `svgLayers` result.
#[derive(Debug, Clone)]
pub struct SvgArt {
    pub width: f64,
    pub height: f64,
    pub layers: Vec<ArtLayer>,
    pub warnings: Vec<String>,
}

impl SvgArt {
    /// The same JSON shape as the JS object.
    pub fn to_json(&self) -> Value {
        obj! {
            "width" => self.width,
            "height" => self.height,
            "layers" => Value::Array(self.layers.iter().map(|l| obj! {
                "name" => l.name.as_str(),
                "items" => Value::Array(l.items.iter().map(|it| obj! {
                    "subs" => Value::Array(it.subs.iter().map(Sub::to_json).collect()),
                    "fill" => Paint::to_json(&it.fill),
                    "stroke" => Paint::to_json(&it.stroke),
                    "strokeWidth" => it.stroke_width,
                    "cap" => it.cap.as_str(),
                    "join" => it.join.as_str(),
                    "evenodd" => it.evenodd,
                    "opacity" => it.opacity,
                }).collect()),
            }).collect()),
            "warnings" => Value::Array(self.warnings.iter().map(|w| Value::String(w.clone())).collect()),
        }
    }
}

struct Ctx<'a> {
    gradients: HashMap<String, &'a SvgNode>,
    warnings: Vec<String>, // a Set: insertion order, no repeats
}

impl<'a> Ctx<'a> {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    // Gradient stops, following href chains (a gradient may borrow another's stops).
    fn stops_of(&self, g: &'a SvgNode) -> Vec<[f64; 5]> {
        let mut seen = HashSet::new();
        let mut node = Some(g);
        while let Some(n) = node {
            if n.children.iter().any(|c| c.tag == "stop") {
                break;
            }
            let href = n.attr("href").or_else(|| n.attr("xlink:href")).unwrap_or("");
            let r = href.strip_prefix('#').unwrap_or(href).to_string();
            if r.is_empty() || seen.contains(&r) {
                break;
            }
            seen.insert(r.clone());
            node = self.gradients.get(&r).copied();
        }
        let Some(node) = node else { return vec![] };
        node.children
            .iter()
            .filter(|c| c.tag == "stop")
            .map(|stop| {
                let mut a = stop.attrs.clone();
                for part in stop.attr("style").unwrap_or("").split(';') {
                    if let (Some(k), Some(v)) = split_style_part(part) {
                        if !k.is_empty() && !v.is_empty() {
                            a.insert(k, v);
                        }
                    }
                }
                let c = parse_color(Some(a.get("stop-color").map(|s| s.as_str()).unwrap_or("#000")));
                let rgba = match c {
                    Some(Color::Rgba(v)) => v,
                    _ => [0.0, 0.0, 0.0, 1.0],
                };
                let off = match a.get("offset") {
                    Some(o) if js::trim(o).ends_with('%') => js::parse_float(o) / 100.0,
                    Some(o) => {
                        let v = js::parse_float(o);
                        if v == 0.0 || v.is_nan() {
                            0.0
                        } else {
                            v
                        }
                    }
                    None => 0.0,
                };
                let so = a.get("stop-opacity").map_or(1.0, |s| js::str_to_number(s));
                [js::min(1.0, js::max(0.0, off)), rgba[0], rgba[1], rgba[2], rgba[3] * so]
            })
            .collect()
    }

    // A gradient as Lottie needs it: start/end points in the drawing's space (radial: centre and a point on the edge).
    fn gradient_of(&self, g: &'a SvgNode, m: &M, bx: &[f64; 4]) -> Option<Gradient> {
        let stops = self.stops_of(g);
        if stops.is_empty() {
            return None;
        }
        let v = |k: &str, d: f64| match g.attr(k) {
            None => d,
            Some(x) if js::trim(x).ends_with('%') => js::parse_float(x) / 100.0,
            Some(x) => js::parse_float(x),
        };
        let radial = g.tag == "radialgradient";
        let [u1, v1, u2, v2] = if radial {
            [v("cx", 0.5), v("cy", 0.5), v("cx", 0.5) + v("r", 0.5), v("cy", 0.5)]
        } else {
            [v("x1", 0.0), v("y1", 0.0), v("x2", 1.0), v("y2", 0.0)]
        };
        let gt = parse_transform(g.attr("gradientTransform"));
        let bm = if g.attr("gradientUnits") == Some("userSpaceOnUse") { IDENTITY } else { [bx[2], 0.0, 0.0, bx[3], bx[0], bx[1]] };
        let to_user = mul(m, &mul(&bm, &gt));
        Some(Gradient { radial, s: apply(&to_user, &[u1, v1]), e: apply(&to_user, &[u2, v2]), stops })
    }

    fn paint_of(&self, st: &Style, key: &str, opacity_key: &str, m: &M, bx: &[f64; 4]) -> Result<Option<Paint>> {
        let c = parse_color(st.get(key));
        let k = st.get(opacity_key).map_or(1.0, js::str_to_number);
        match c {
            Some(Color::Ref(Some(r))) => {
                let grad = self.gradients.get(&r).and_then(|g| self.gradient_of(g, m, bx));
                Ok(grad.map(|gradient| Paint::Gradient { gradient, alpha: k }))
            }
            None | Some(Color::None) => Ok(None),
            Some(Color::Ref(None)) => Err(anyhow!("c.slice is not a function")),
            Some(Color::Rgba(c)) => Ok(Some(Paint::Solid([c[0], c[1], c[2], c[3] * k]))),
        }
    }

    fn items(&mut self, n: &'a SvgNode, m: &M, st: &Style, out: &mut Vec<Item>) -> Result<()> {
        if is_skip(&n.tag) {
            return Ok(());
        }
        let own = own_style(n);
        let mut s = st.merged(&own);
        if s.get("display") == Some("none") || s.get("visibility") == Some("hidden") {
            return Ok(());
        }
        let mm = if has(n, "transform") { mul(m, &parse_transform(n.attr("transform"))) } else { *m };
        let opacity = st.opacity.unwrap_or(1.0) * own.get("opacity").map_or(1.0, |o| js::str_to_number(o));
        s.opacity = Some(opacity);
        if has(n, "clip-path") || has(n, "mask") {
            self.warn("clip-path / mask ignored");
        }
        if has(n, "filter") {
            self.warn("filters (blur, shadow) ignored");
        }
        if n.tag == "use" || n.tag == "image" || n.tag == "text" {
            self.warn(&format!("<{}> not supported — outline it in Figma first", n.tag));
            return Ok(());
        }
        if is_drawable(&n.tag) {
            let d = match shape_to_path(n) {
                Some(d) if !d.is_empty() => d,
                _ => return Ok(()),
            };
            let local = parse_path(&d);
            let subs: Vec<Sub> = local
                .iter()
                .map(|sp| Sub { closed: sp.closed, pts: sp.pts.iter().map(|p| apply(&mm, p)).collect(), segs: sp.segs.iter().map(|[a, b]| [apply(&mm, a), apply(&mm, b)]).collect() })
                .collect();
            if subs.is_empty() {
                return Ok(());
            }
            let lb = bounds(local.iter());
            let or1 = |v: f64| if v == 0.0 || v.is_nan() { 1.0 } else { v };
            let bx = [lb.cx - lb.w / 2.0, lb.cy - lb.h / 2.0, or1(lb.w), or1(lb.h)];
            let fill = if s.get("fill").is_none() { Some(Paint::Solid([0.0, 0.0, 0.0, 1.0])) } else { self.paint_of(&s, "fill", "fill-opacity", &mm, &bx)? };
            let stroke = self.paint_of(&s, "stroke", "stroke-opacity", &mm, &bx)?;
            let sw = js::parse_float(s.get("stroke-width").unwrap_or("1")) * (mm[0] * mm[3] - mm[1] * mm[2]).abs().sqrt();
            if fill.is_none() && stroke.is_none() {
                return Ok(());
            }
            out.push(Item {
                subs,
                fill,
                stroke,
                stroke_width: sw,
                cap: s.get("stroke-linecap").unwrap_or("butt").to_string(),
                join: s.get("stroke-linejoin").unwrap_or("miter").to_string(),
                evenodd: s.get("fill-rule") == Some("evenodd"),
                opacity,
            });
            return Ok(());
        }
        for c in &n.children {
            self.items(c, &mm, &s, out)?;
        }
        Ok(())
    }
}

fn norm_name(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ('ก'..='๙').contains(c)).collect()
}

/// `svgLayers(text, { split })`: `split` names groups to open — their children become layers of their
/// own (a nested layer that animates in Figma Motion lives inside a group in the SVG export).
pub fn svg_layers(text: &str, split: &[String]) -> Result<SvgArt> {
    let split: HashSet<String> = split.iter().map(|s| norm_name(s)).collect();
    let svg = parse_svg(text)?;
    let mut gradients = HashMap::new();
    fn walk<'a>(n: &'a SvgNode, g: &mut HashMap<String, &'a SvgNode>) {
        if n.tag.ends_with("gradient") {
            if let Some(id) = n.attr("id") {
                if !id.is_empty() {
                    g.insert(id.to_string(), n);
                }
            }
        }
        for c in &n.children {
            walk(c, g);
        }
    }
    walk(&svg, &mut gradients);
    let mut cx = Ctx { gradients, warnings: vec![] };
    let vb: Vec<f64> = split_ws_comma(svg.attr("viewBox").unwrap_or("")).into_iter().map(js::str_to_number).collect();
    let or = |a: f64, b: Option<f64>, c: f64| {
        if a != 0.0 && !a.is_nan() {
            a
        } else {
            match b {
                Some(b) if b != 0.0 && !b.is_nan() => b,
                _ => c,
            }
        }
    };
    let width = or(svg.attr("width").map_or(f64::NAN, js::parse_float), vb.get(2).copied(), 100.0);
    let height = or(svg.attr("height").map_or(f64::NAN, js::parse_float), vb.get(3).copied(), 100.0);
    // viewBox → output size
    let mut base = IDENTITY;
    let truthy = |v: f64| v != 0.0 && !v.is_nan();
    if vb.len() == 4 && truthy(vb[2]) && truthy(vb[3]) {
        base = mul(&[width / vb[2], 0.0, 0.0, height / vb[3], 0.0, 0.0], &[1.0, 0.0, 0.0, 1.0, -vb[0], -vb[1]]);
    }

    // Top level: unwrap single wrapping groups (Figma wraps the frame's content in one <g>).
    let mut top: Vec<&SvgNode> = svg.children.iter().filter(|c| !is_skip(&c.tag)).collect();
    let mut m0 = base;
    let mut st0 = Style { props: own_style(&svg), opacity: None };
    while top.len() == 1 && top[0].tag == "g" && top[0].children.iter().any(|c| !is_skip(&c.tag)) {
        let g = top[0];
        if has(g, "transform") {
            m0 = mul(&m0, &parse_transform(g.attr("transform")));
        }
        st0 = st0.merged(&own_style(g));
        if has(g, "clip-path") || has(g, "mask") {
            cx.warn("clip-path / mask ignored");
        }
        top = g.children.iter().filter(|c| !is_skip(&c.tag)).collect();
    }
    // open the groups named in split (recursively), keeping their transform and style for the children
    let mut flat: Vec<(&SvgNode, M, Style)> = vec![];
    fn open<'a>(n: &'a SvgNode, m: M, st: Style, split: &HashSet<String>, flat: &mut Vec<(&'a SvgNode, M, Style)>) {
        let name = n.attr("data-name").or_else(|| n.attr("id"));
        let named = name.is_some_and(|nm| !nm.is_empty() && split.contains(&norm_name(nm)));
        if named && n.children.iter().any(|c| !is_skip(&c.tag)) && !is_drawable(&n.tag) {
            let mm = if has(n, "transform") { mul(&m, &parse_transform(n.attr("transform"))) } else { m };
            let own = own_style(n);
            let mut s = st.merged(&own);
            s.opacity = Some(st.opacity.unwrap_or(1.0) * own.get("opacity").map_or(1.0, |o| js::str_to_number(o)));
            for c in &n.children {
                if !is_skip(&c.tag) {
                    open(c, mm, s.clone(), split, flat);
                }
            }
        } else {
            flat.push((n, m, st));
        }
    }
    for n in &top {
        open(n, m0, st0.clone(), &split, &mut flat);
    }
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut layers: Vec<ArtLayer> = vec![];
    for (n, m, st) in flat {
        let mut out = vec![];
        cx.items(n, &m, &st, &mut out)?;
        if out.is_empty() {
            continue;
        }
        // Figma exports the 2nd "Dot" as id "Dot_2": a "dot*" track reaches both.
        let want = n.attr("data-name").or_else(|| n.attr("id")).map(|s| s.to_string()).unwrap_or_else(|| format!("{}-{}", n.tag, layers.len() + 1));
        let seen = used.get(&want).copied().unwrap_or(0) + 1;
        used.insert(want.clone(), seen);
        let mut name = if seen == 1 { want.clone() } else { format!("{want}_{seen}") };
        while layers.iter().any(|l| l.name == name) {
            name.push('_'); // an id that already looks like "Dot_2"
        }
        layers.push(ArtLayer { name, items: out });
    }
    Ok(SvgArt { width, height, layers, warnings: cx.warnings })
}

// ---------------------------------------------------------------- motion
const EASE_NAMES: [&str; 12] = ["linear", "ease", "in", "out", "in-out", "standard", "decelerate", "accelerate", "emphasized", "back", "back-in", "anticipate"];
fn ease_named(name: &str) -> Option<[f64; 4]> {
    Some(match name {
        "linear" => [0.0, 0.0, 1.0, 1.0],
        "ease" => [0.25, 0.1, 0.25, 1.0],
        "in" => [0.42, 0.0, 1.0, 1.0],
        "out" => [0.0, 0.0, 0.58, 1.0],
        "in-out" => [0.42, 0.0, 0.58, 1.0],
        "standard" => [0.2, 0.0, 0.0, 1.0],
        "decelerate" => [0.0, 0.0, 0.0, 1.0],
        "accelerate" => [0.3, 0.0, 1.0, 1.0],
        "emphasized" => [0.05, 0.7, 0.1, 1.0],
        "back" => [0.34, 1.56, 0.64, 1.0],
        "back-in" => [0.36, 0.0, 0.66, -0.56],
        "anticipate" => [0.68, -0.6, 0.32, 1.6],
        _ => return None,
    })
}
const LINEAR: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
const STANDARD: [f64; 4] = [0.2, 0.0, 0.0, 1.0];

/// `EASINGS`: the ease names `create_lottie` accepts.
pub fn easings() -> Vec<&'static str> {
    let mut v = EASE_NAMES.to_vec();
    v.extend(["spring", "bounce", "hold"]);
    v
}

fn pad4(v: Vec<f64>) -> [f64; 4] {
    [v.first().copied().unwrap_or(f64::NAN), v.get(1).copied().unwrap_or(f64::NAN), v.get(2).copied().unwrap_or(f64::NAN), v.get(3).copied().unwrap_or(f64::NAN)]
}

fn ease_of(e: Option<&Value>) -> Option<[f64; 4]> {
    if let Some(Value::Array(a)) = e {
        if a.len() == 4 {
            return Some(pad4(a.iter().map(js::number).collect()));
        }
    }
    let s = match js::nullish(e) {
        None => String::new(),
        Some(v) => js::string(v),
    };
    if let Some(m) = re!(r"^cubic\(([^)]+)\)$").captures(&s) {
        return Some(pad4(m[1].split(',').map(js::str_to_number).collect()));
    }
    let key = match js::nullish(e) {
        None => "standard".to_string(),
        Some(v) => js::string(v),
    };
    ease_named(&key)
}

// Damped spring 0→1 sampled (Lottie has no spring curve).
fn spring_curve(t: f64) -> f64 {
    let (stiffness, damping) = (170.0f64, 16.0f64);
    let w0 = stiffness.sqrt();
    let z = damping / (2.0 * stiffness.sqrt());
    if z >= 1.0 {
        return 1.0 - exp(-w0 * t) * (1.0 + w0 * t);
    }
    let wd = w0 * (1.0 - z * z).sqrt();
    1.0 - exp(-z * w0 * t) * (cos(wd * t) + ((z * w0) / wd) * sin(wd * t))
}
fn bounce_curve(mut t: f64) -> f64 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        return n * t * t;
    }
    if t < 2.0 / d {
        t -= 1.5 / d;
        return n * t * t + 0.75;
    }
    if t < 2.5 / d {
        t -= 2.25 / d;
        return n * t * t + 0.9375;
    }
    t -= 2.625 / d;
    n * t * t + 0.984375
}

const PROPS: [&str; 7] = ["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"];

/// A key's value: what the spec gave (JSON, maybe undefined) or a computed in-between.
#[derive(Debug, Clone)]
enum V {
    Json(Option<Value>),
    Num(f64),
    Arr(Vec<f64>),
    Str(String),
}

impl V {
    fn is_array(&self) -> bool {
        matches!(self, V::Arr(_) | V::Json(Some(Value::Array(_))))
    }
    fn number(&self) -> f64 {
        match self {
            V::Json(v) => js::to_number(v.as_ref()),
            V::Num(x) => *x,
            V::Arr(a) => js::str_to_number(&a.iter().map(|x| js::num_str(*x)).collect::<Vec<_>>().join(",")),
            V::Str(s) => js::str_to_number(s),
        }
    }
    /// `v[i]` as a number (undefined → NaN)
    fn idx(&self, i: usize) -> f64 {
        match self {
            V::Json(Some(Value::Array(a))) => js::to_number(a.get(i)),
            V::Arr(a) => a.get(i).copied().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
    fn len(&self) -> usize {
        match self {
            V::Json(Some(Value::Array(a))) => a.len(),
            V::Arr(a) => a.len(),
            _ => 0,
        }
    }
    fn to_str(&self) -> String {
        match self {
            V::Json(v) => js::to_string(v.as_ref()),
            V::Num(x) => js::num_str(*x),
            V::Arr(a) => a.iter().map(|x| js::num_str(*x)).collect::<Vec<_>>().join(","),
            V::Str(s) => s.clone(),
        }
    }
    fn is_nullish(&self) -> bool {
        matches!(self, V::Json(None) | V::Json(Some(Value::Null)))
    }
}

fn rest_of(prop: &str) -> Option<V> {
    match prop {
        "opacity" | "scale" | "draw" => Some(V::Num(1.0)),
        "rotate" => Some(V::Num(0.0)),
        "move" => Some(V::Arr(vec![0.0, 0.0])),
        _ => None,
    }
}

/// A layer with its centre (the pivot) and size.
struct Placed {
    name: String,
    items: Vec<Item>,
    cx: f64,
    cy: f64,
}

// Value in spec units → Lottie units for one layer.
fn to_lottie(prop: &str, v: &V, l: &Placed) -> Result<Vec<f64>> {
    Ok(match prop {
        "opacity" => vec![round(v.number() * 100.0)],
        "scale" => {
            let (a, b) = if v.is_array() { (v.idx(0), v.idx(1)) } else { (v.number(), v.number()) };
            vec![round(a * 100.0), round(b * 100.0), 100.0]
        }
        "move" => {
            let (a, b) = if v.is_array() { (v.idx(0), v.idx(1)) } else { (v.number(), 0.0) };
            vec![round(l.cx + a), round(l.cy + b), 0.0]
        }
        "rotate" => vec![round(v.number())],
        "draw" => vec![round(v.number() * 100.0)],
        "fill" | "stroke" => match parse_color(color_input(v).as_deref()) {
            Some(Color::Rgba(c)) => c.iter().map(|x| round(*x)).collect(),
            _ => return Err(anyhow!("{prop}: \"{}\" is not a colour", v.to_str())),
        },
        _ => return Err(anyhow!("Unknown prop \"{prop}\" — use {}", PROPS.join(", "))),
    })
}

// parseColor(v) takes String(v) unless v is null / undefined
fn color_input(v: &V) -> Option<String> {
    if v.is_nullish() {
        None
    } else {
        Some(v.to_str())
    }
}

fn match_name(pattern: &str, name: &str) -> bool {
    let norm = |s: &str| -> String { s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '*' || ('ก'..='๙').contains(c)).collect() };
    let p = norm(pattern);
    let n = norm(name);
    if p == "all" || p == "*" {
        return true;
    }
    if p.contains('*') {
        let pat = format!("^{}$", p.split('*').map(regex::escape).collect::<Vec<_>>().join(".*"));
        return Regex::new(&pat).map(|r| r.is_match(&n)).unwrap_or(false);
    }
    p == n
}

#[derive(Debug, Clone)]
struct Key {
    t: f64,
    v: V,
    ease: [f64; 4],
    hold: bool,
}

type LayerKeys = Vec<(String, Vec<Key>)>; // prop → keys, in first-use order

fn prop_keys<'a>(lk: &'a mut LayerKeys, prop: &str) -> &'a mut Vec<Key> {
    if let Some(i) = lk.iter().position(|(p, _)| p == prop) {
        return &mut lk[i].1;
    }
    lk.push((prop.to_string(), vec![]));
    &mut lk.last_mut().unwrap().1
}

// Interpolate spec values (colours as rgba arrays) for sampled curves.
fn lerp_values(prop: &str, a: &V, b: &V, t: f64) -> V {
    if prop == "fill" || prop == "stroke" {
        let ca = match parse_color(color_input(a).as_deref()) {
            Some(Color::Rgba(c)) => c,
            _ => [f64::NAN; 4],
        };
        let cb = match parse_color(color_input(b).as_deref()) {
            Some(Color::Rgba(c)) => c,
            _ => [f64::NAN; 4],
        };
        let parts: Vec<String> = (0..4)
            .map(|i| {
                let x = ca[i] + (cb[i] - ca[i]) * t;
                if i < 3 {
                    js::num_str(js::round(x * 255.0))
                } else {
                    js::num_str(round(x))
                }
            })
            .collect();
        return V::Str(format!("rgba({})", parts.join(",")));
    }
    if a.is_array() || b.is_array() {
        let av: Vec<f64> = if a.is_array() { (0..a.len()).map(|i| a.idx(i)).collect() } else { vec![a.number(), a.number()] };
        let bv = |i: usize| if b.is_array() { b.idx(i) } else if i < 2 { b.number() } else { f64::NAN };
        return V::Arr(av.iter().enumerate().map(|(i, x)| x + (bv(i) - x) * t).collect());
    }
    let (x, y) = (a.number(), b.number());
    V::Num(x + (y - x) * t)
}

// tracks → per layer { [prop]: [{ t, v, ease, hold }] } in frames.
fn keyframes(tracks: &[Value], layers: &[Placed], fps: f64, total_ms: f64, warnings: &mut Vec<String>) -> Result<Vec<LayerKeys>> {
    let mut out: Vec<LayerKeys> = layers.iter().map(|_| vec![]).collect();
    let frame = |ms: f64| js::round((ms / 1000.0) * fps);
    for (ti, tr) in tracks.iter().enumerate() {
        let trv = Some(tr);
        let prop = match js::get(trv, "prop") {
            Some(Value::String(p)) if PROPS.contains(&p.as_str()) => p.clone(),
            other => return Err(anyhow!("track {}: unknown prop \"{}\" — use {}", ti + 1, js::to_string(other), PROPS.join(", "))),
        };
        let pattern = js::nullish(js::get(trv, "layer")).map_or("all".to_string(), js::string);
        let targets: Vec<usize> = (0..layers.len()).filter(|&i| match_name(&pattern, &layers[i].name)).collect();
        if targets.is_empty() {
            warnings.push(format!(
                "track {}: no layer named \"{}\" (layers: {})",
                ti + 1,
                js::to_string(js::get(trv, "layer")),
                layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")
            ));
            continue;
        }
        let tr_ease = js::get(trv, "ease").cloned();
        for (k, &li) in targets.iter().enumerate() {
            let stagger = js::nullish(js::get(trv, "stagger")).map_or(0.0, js::number);
            let off = stagger * k as f64;
            // keys: (ms, v, ease)
            let mut keys: Vec<(f64, V, Option<Value>)> = vec![];
            match js::get(trv, "keys") {
                Some(Value::Array(ks)) if ks.len() >= 2 => {
                    for kk in ks {
                        let a = kk.as_array();
                        let at = |i: usize| a.and_then(|a| a.get(i));
                        let ms = js::to_number(at(0)) + off;
                        let ease = js::nullish(at(2)).cloned().or_else(|| tr_ease.clone().filter(|v| !v.is_null()));
                        keys.push((ms, V::Json(at(1).cloned()), ease));
                    }
                }
                _ => {
                    let start0 = js::nullish(js::get(trv, "start")).map_or(0.0, js::number);
                    let start = start0 + off;
                    let end = match js::nullish(js::get(trv, "end")) {
                        Some(e) => js::number(e),
                        None => match js::nullish(js::get(trv, "duration")) {
                            Some(d) => start0 + js::number(d),
                            None => total_ms,
                        },
                    } + off;
                    let rest = if prop == "fill" || prop == "stroke" { None } else { rest_of(&prop) };
                    let from = js::nullish(js::get(trv, "from")).map(|v| V::Json(Some(v.clone()))).or_else(|| rest.clone());
                    let to = js::nullish(js::get(trv, "to")).map(|v| V::Json(Some(v.clone()))).or(rest);
                    let (Some(from), Some(to)) = (from, to) else {
                        return Err(anyhow!("track {}: {} needs from and to", ti + 1, prop));
                    };
                    keys.push((start, from, tr_ease.clone()));
                    keys.push((end, to, None));
                }
            }
            let list = prop_keys(&mut out[li], &prop);
            if let Some(last) = list.last_mut() {
                last.hold = true; // a later track on the same prop starts from its own "from", no drift in between
            }
            for i in 0..keys.len() {
                let (a_ms, a_v, a_ease) = &keys[i];
                let b = keys.get(i + 1);
                let ease: Value = a_ease.clone().filter(|v| !v.is_null()).or_else(|| tr_ease.clone().filter(|v| !v.is_null())).unwrap_or_else(|| Value::String("standard".into()));
                let ease_s = ease.as_str();
                if let (Some(b), Some(es @ ("spring" | "bounce"))) = (b, ease_s) {
                    // sample the curve: one key every 2 frames, linear in between
                    let (f0, f1) = (frame(*a_ms), frame(b.0));
                    let n = js::max(2.0, js::round((f1 - f0) / 2.0));
                    let mut s = 0.0;
                    while s < n {
                        // the spring settles in ~0.6s of its own time: stretch that over the track
                        let p = s / n;
                        let eased = if es == "spring" { spring_curve(p * 0.6) } else { bounce_curve(p) };
                        list.push(Key { t: f0 + js::round((f1 - f0) * p), v: lerp_values(&prop, a_v, &b.1, eased), ease: LINEAR, hold: false });
                        s += 1.0;
                    }
                } else if b.is_some() && ease_s == Some("hold") {
                    list.push(Key { t: frame(*a_ms), v: a_v.clone(), ease: LINEAR, hold: true }); // keep the value, jump at the next key
                } else {
                    let curve = if b.is_some() { ease_of(Some(&ease)) } else { Some(LINEAR) }; // the last key has no segment after it
                    if curve.is_none() {
                        warnings.push(format!(
                            "track {}: unknown ease \"{}\" — used standard (choices: {}, cubic(x1,y1,x2,y2))",
                            ti + 1,
                            js::string(&ease),
                            easings().join(", ")
                        ));
                    }
                    list.push(Key { t: frame(*a_ms), v: a_v.clone(), ease: curve.unwrap_or(STANDARD), hold: false });
                }
            }
        }
    }
    Ok(out)
}

fn lottie_value(k: Vec<f64>) -> Value {
    if k.len() == 1 {
        num(k[0])
    } else {
        nums(&k)
    }
}

// A value that does not move: 1-D properties are plain numbers (opacity: 100, not [100]).
fn still(prop: &str, v: &V, l: &Placed) -> Result<Value> {
    let k = to_lottie(prop, v, l)?;
    Ok(obj! { "a" => 0i64, "k" => lottie_value(k) })
}

fn animated(prop: &str, keys: &[Key], l: &Placed) -> Result<Value> {
    let mut sorted: Vec<&Key> = keys.iter().collect();
    sorted.sort_by(|a, b| js::cmp_num(a.t - b.t));
    // two keys on the same frame: keep the later one
    let ks: Vec<&Key> = sorted.iter().enumerate().filter(|(i, k)| !sorted.get(i + 1).is_some_and(|n| n.t == k.t)).map(|(_, k)| *k).collect();
    if ks.len() == 1 {
        return still(prop, &ks[0].v, l);
    }
    let mut out = vec![];
    for (i, k) in ks.iter().enumerate() {
        let s = nums(&to_lottie(prop, &k.v, l)?);
        if i == ks.len() - 1 {
            out.push(obj! { "t" => k.t, "s" => s });
            continue;
        }
        let [x1, y1, x2, y2] = k.ease;
        if k.hold {
            out.push(obj! { "t" => k.t, "s" => s, "h" => 1i64 });
        } else {
            out.push(obj! {
                "t" => k.t, "s" => s,
                "o" => obj! { "x" => nums(&[x1]), "y" => nums(&[y1]) },
                "i" => obj! { "x" => nums(&[x2]), "y" => nums(&[y2]) },
            });
        }
    }
    Ok(obj! { "a" => 1i64, "k" => Value::Array(out) })
}

// ---------------------------------------------------------------- compile
fn cap_of(s: &str) -> i64 {
    match s {
        "butt" => 1,
        "round" => 2,
        "square" => 3,
        _ => 1,
    }
}
fn join_of(s: &str) -> i64 {
    match s {
        "miter" => 1,
        "round" => 2,
        "bevel" => 3,
        _ => 1,
    }
}

fn path_shape(sp: &Sub, cx: f64, cy: f64) -> Value {
    let n = sp.pts.len();
    let v: Vec<Value> = sp.pts.iter().map(|p| nums(&[round(p[0] - cx), round(p[1] - cy)])).collect();
    let o: Vec<Value> = sp
        .pts
        .iter()
        .enumerate()
        .map(|(i, p)| match sp.segs.get(i) {
            Some(seg) => nums(&[round(seg[0][0] - p[0]), round(seg[0][1] - p[1])]),
            None => nums(&[0.0, 0.0]),
        })
        .collect();
    let iv: Vec<Value> = sp
        .pts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let seg = if i == 0 {
                if sp.closed {
                    sp.segs.get(n - 1)
                } else {
                    None
                }
            } else {
                sp.segs.get(i - 1)
            };
            match seg {
                Some(seg) => nums(&[round(seg[1][0] - p[0]), round(seg[1][1] - p[1])]),
                None => nums(&[0.0, 0.0]),
            }
        })
        .collect();
    obj! {
        "ty" => "sh",
        "ks" => obj! {
            "a" => 0i64,
            // vertices relative to the layer anchor; tangents relative to their vertex
            "k" => obj! { "c" => sp.closed, "v" => Value::Array(v), "o" => Value::Array(o), "i" => Value::Array(iv) },
        },
    }
}

// Gradient fill (gf) / stroke (gs): colour stops, then opacity stops when any stop is see-through.
fn gradient_paint(ty: &str, g: &Gradient, alpha: f64, it: &Item, l: &Placed) -> Obj {
    let mut k: Vec<f64> = g.stops.iter().flat_map(|s| [s[0], s[1], s[2], s[3]]).collect();
    if g.stops.iter().any(|x| x[4] < 1.0) {
        k.extend(g.stops.iter().flat_map(|s| [s[0], s[4]]));
    }
    let k: Vec<f64> = k.into_iter().map(round).collect();
    let v = obj! {
        "ty" => ty, "nm" => if ty == "gf" { "fill" } else { "stroke" }, "o" => obj! { "a" => 0i64, "k" => round(alpha * it.opacity * 100.0) }, "r" => if it.evenodd { 2i64 } else { 1 }, "bm" => 0i64,
        "g" => obj! { "p" => g.stops.len(), "k" => obj! { "a" => 0i64, "k" => nums(&k) } },
        "s" => obj! { "a" => 0i64, "k" => nums(&[round(g.s[0] - l.cx), round(g.s[1] - l.cy)]) }, "e" => obj! { "a" => 0i64, "k" => nums(&[round(g.e[0] - l.cx), round(g.e[1] - l.cy)]) },
        "t" => if g.radial { 2i64 } else { 1 }, "h" => obj! { "a" => 0i64, "k" => 0i64 }, "a" => obj! { "a" => 0i64, "k" => 0i64 },
    };
    match v {
        Value::Object(o) => o,
        _ => unreachable!(),
    }
}

struct Bounds {
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
}

fn bounds<'a>(subs: impl Iterator<Item = &'a Sub>) -> Bounds {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for sp in subs {
        for p in sp.pts.iter().chain(sp.segs.iter().flat_map(|s| s.iter())) {
            x0 = js::min(x0, p[0]);
            y0 = js::min(y0, p[1]);
            x1 = js::max(x1, p[0]);
            y1 = js::max(y1, p[1]);
        }
    }
    Bounds { cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, w: x1 - x0, h: y1 - y0 }
}

/// `compileLottie(spec)`.
///
/// spec: `{ name?, width?, height?, duration (ms), fps = 60, svg (markup), split?, tracks: [{ layer, prop, from?, to?, start?, end?, duration?, keys?, ease?, stagger? }] }`.
/// Returns `{ lottie, summary, warnings }`.
pub fn compile_lottie(spec: &Value) -> Result<Value> {
    let sp = Some(spec);
    let fps_v = js::nullish(js::get(sp, "fps")).cloned().unwrap_or(Value::from(60));
    let fps = js::number(&fps_v);
    let duration = js::nullish(js::get(sp, "duration")).map_or(1000.0, js::number);
    if duration.is_nan() || duration <= 0.0 {
        return Err(anyhow!("duration (ms) must be > 0"));
    }
    let svg = match js::get(sp, "svg") {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => return Err(anyhow!("Cannot read properties of {} (reading 'matchAll')", js::to_string(js::get(sp, "svg")))),
        Some(_) => return Err(anyhow!("text.matchAll is not a function")),
    };
    let split: Vec<String> = match js::nullish(js::get(sp, "split")) {
        None => vec![],
        Some(Value::Array(a)) => a.iter().map(js::string).collect(),
        Some(_) => return Err(anyhow!("(opts.split ?? []).map is not a function")),
    };
    let art = svg_layers(&svg, &split)?;
    let width_v = js::nullish(js::get(sp, "width")).cloned().unwrap_or(num(art.width));
    let height_v = js::nullish(js::get(sp, "height")).cloned().unwrap_or(num(art.height));
    let (width, height) = (js::number(&width_v), js::number(&height_v));
    let (sx, sy) = (width / art.width, height / art.height);
    let mut warnings = art.warnings.clone();
    let layers: Vec<Placed> = art
        .layers
        .into_iter()
        .map(|mut l| {
            if sx != 1.0 || sy != 1.0 {
                for it in &mut l.items {
                    for s in &mut it.subs {
                        s.pts = s.pts.iter().map(|[x, y]| [x * sx, y * sy]).collect();
                        s.segs = s.segs.iter().map(|seg| [[seg[0][0] * sx, seg[0][1] * sy], [seg[1][0] * sx, seg[1][1] * sy]]).collect();
                    }
                }
            }
            let b = bounds(l.items.iter().flat_map(|it| it.subs.iter()));
            Placed { name: l.name, items: l.items, cx: b.cx, cy: b.cy }
        })
        .collect();
    if layers.is_empty() {
        return Err(anyhow!("The SVG has nothing to draw (no filled or stroked shapes)"));
    }
    let frames = js::max(1.0, js::round((duration / 1000.0) * fps));
    let tracks: Vec<Value> = match js::nullish(js::get(sp, "tracks")) {
        None => vec![],
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(anyhow!("tracks.forEach is not a function")),
    };
    let keys = keyframes(&tracks, &layers, fps, duration, &mut warnings)?;

    let mut out = vec![];
    for (idx, l) in layers.iter().enumerate() {
        let k = &keys[idx];
        let kget = |name: &str| k.iter().find(|(p, _)| p == name).map(|(_, v)| v);
        let prop = |name: &str, rest: V| -> Result<Value> {
            match kget(name) {
                Some(keys) => animated(name, keys, l),
                None => still(name, &rest, l),
            }
        };
        let mut groups = vec![];
        for (gi, it) in l.items.iter().enumerate() {
            let mut its: Vec<Value> = it.subs.iter().map(|s| path_shape(s, l.cx, l.cy)).collect();
            if let Some(dk) = kget("draw") {
                its.push(obj! { "ty" => "tm", "s" => obj! { "a" => 0i64, "k" => 0i64 }, "e" => animated("draw", dk, l)?, "o" => obj! { "a" => 0i64, "k" => 0i64 }, "m" => 1i64, "nm" => "draw" });
            }
            match &it.fill {
                Some(Paint::Gradient { gradient, alpha }) => its.push(Value::Object(gradient_paint("gf", gradient, *alpha, it, l))),
                Some(Paint::Solid(f)) => {
                    let c = match kget("fill") {
                        Some(fk) => animated("fill", fk, l)?,
                        None => obj! { "a" => 0i64, "k" => nums(&[round(f[0]), round(f[1]), round(f[2]), 1.0]) },
                    };
                    its.push(obj! { "ty" => "fl", "c" => c, "o" => obj! { "a" => 0i64, "k" => round(f[3] * it.opacity * 100.0) }, "r" => if it.evenodd { 2i64 } else { 1 }, "nm" => "fill" });
                }
                None => {}
            }
            match &it.stroke {
                Some(Paint::Gradient { gradient, alpha }) => {
                    let mut g = gradient_paint("gs", gradient, *alpha, it, l);
                    js::put(&mut g, "w", obj! { "a" => 0i64, "k" => round(it.stroke_width) });
                    js::put(&mut g, "lc", cap_of(&it.cap));
                    js::put(&mut g, "lj", join_of(&it.join));
                    js::put(&mut g, "ml", 4i64);
                    its.push(Value::Object(g));
                }
                Some(Paint::Solid(s)) => {
                    let c = match kget("stroke") {
                        Some(sk) => animated("stroke", sk, l)?,
                        None => obj! { "a" => 0i64, "k" => nums(&[round(s[0]), round(s[1]), round(s[2]), 1.0]) },
                    };
                    its.push(obj! {
                        "ty" => "st", "c" => c, "o" => obj! { "a" => 0i64, "k" => round(s[3] * it.opacity * 100.0) }, "w" => obj! { "a" => 0i64, "k" => round(it.stroke_width) },
                        "lc" => cap_of(&it.cap), "lj" => join_of(&it.join), "ml" => 4i64, "nm" => "stroke",
                    });
                }
                None => {}
            }
            its.push(obj! {
                "ty" => "tr", "p" => obj! { "a" => 0i64, "k" => nums(&[0.0, 0.0]) }, "a" => obj! { "a" => 0i64, "k" => nums(&[0.0, 0.0]) }, "s" => obj! { "a" => 0i64, "k" => nums(&[100.0, 100.0]) },
                "r" => obj! { "a" => 0i64, "k" => 0i64 }, "o" => obj! { "a" => 0i64, "k" => 100i64 }, "sk" => obj! { "a" => 0i64, "k" => 0i64 }, "sa" => obj! { "a" => 0i64, "k" => 0i64 },
            });
            groups.push(obj! { "ty" => "gr", "nm" => format!("{}-{}", l.name, gi + 1), "it" => Value::Array(its) });
        }
        groups.reverse(); // Lottie draws the first group on top; SVG draws the last on top
        out.push(obj! {
            "ddd" => 0i64, "ind" => idx + 1, "ty" => 4i64, "nm" => l.name.as_str(), "sr" => 1i64, "ao" => 0i64, "ip" => 0i64, "op" => frames, "st" => 0i64, "bm" => 0i64,
            "ks" => obj! {
                "o" => prop("opacity", V::Num(1.0))?,
                "r" => prop("rotate", V::Num(0.0))?,
                "p" => prop("move", V::Arr(vec![0.0, 0.0]))?,
                "a" => obj! { "a" => 0i64, "k" => nums(&[0.0, 0.0, 0.0]) },
                "s" => prop("scale", V::Num(1.0))?,
            },
            "shapes" => Value::Array(groups),
        });
    }
    out.reverse();
    let nm = js::nullish(js::get(sp, "name")).cloned().unwrap_or_else(|| "animation".into());
    let (w, h) = (js::round(width), js::round(height));
    let lottie = obj! {
        "v" => "5.7.4", "fr" => fps_v.clone(), "ip" => 0i64, "op" => frames, "w" => w, "h" => h, "nm" => nm.clone(),
        "ddd" => 0i64, "assets" => Value::Array(vec![]), "layers" => Value::Array(out), "markers" => Value::Array(vec![]),
    };
    let animated_layers: Vec<String> =
        layers.iter().zip(&keys).filter(|(_, k)| !k.is_empty()).map(|(l, k)| format!("{}: {}", l.name, k.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", "))).collect();
    let summary = [
        format!(
            "{} · {}×{} · {}ms @ {}fps ({} frames) · {} layer(s)",
            js::string(&nm),
            js::num_str(w),
            js::num_str(h),
            js::num_str(duration),
            js::string(&fps_v),
            js::num_str(frames),
            layers.len()
        ),
        format!("layers: {}", layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")),
        if animated_layers.is_empty() { "nothing animated (no track matched a layer)".to_string() } else { format!("animated: {}", animated_layers.join(" · ")) },
    ];
    Ok(obj! { "lottie" => lottie, "summary" => summary.join("\n"), "warnings" => Value::Array(warnings.into_iter().map(Value::String).collect()) })
}
