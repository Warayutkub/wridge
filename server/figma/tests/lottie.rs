//! Port of legacy-node/test/lottie.test.mjs.
use serde_json::{json, Value};
use wridge_figma::lottie::{compile_lottie, parse_color, parse_path, svg_layers, Color, Paint};

const CHECK: &str = r##"<svg width="120" height="120" viewBox="0 0 120 120" fill="none" xmlns="http://www.w3.org/2000/svg">
<g clip-path="url(#c)">
<circle id="Circle" cx="60" cy="60" r="50" fill="#22C55E"/>
<path id="Check" d="M38 62l14 14 30-32" stroke="white" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>
<rect id="Dot" x="10" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
<rect id="Dot_2" x="104" y="10" width="6" height="6" rx="3" fill="#ffffff"/>
</g>
<defs><clipPath id="c"><rect width="120" height="120"/></clipPath></defs>
</svg>"##;

fn layer<'a>(lottie: &'a Value, name: &str) -> &'a Value {
    lottie["layers"].as_array().unwrap().iter().find(|l| l["nm"] == name).unwrap()
}
fn warnings(out: &Value) -> Vec<String> {
    out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

#[test]
fn path_data_relative_lines_arcs_and_closing() {
    let sp = &parse_path("M10 10 h20 v20 h-20 z")[0];
    assert!(sp.closed);
    assert_eq!(sp.pts, vec![[10.0, 10.0], [30.0, 10.0], [30.0, 30.0], [10.0, 30.0]]);
    assert_eq!(sp.segs.len(), 4);
    let arc = &parse_path("M0 0 A10 10 0 0 1 20 0")[0];
    assert!(arc.segs.len() >= 2, "half circle → 2 cubic pieces");
    let last = arc.pts.last().unwrap();
    assert_eq!([last[0].round() + 0.0, last[1].round() + 0.0], [20.0, 0.0]);
    let q = &parse_path("M0 0 Q10 10 20 0 T40 0")[0];
    assert_eq!(*q.pts.last().unwrap(), [40.0, 0.0]);
}

#[test]
fn colours() {
    assert_eq!(parse_color(Some("#fff")), Some(Color::Rgba([1.0, 1.0, 1.0, 1.0])));
    assert_eq!(parse_color(Some("none")), Some(Color::None));
    assert_eq!(parse_color(Some("rgba(255,0,0,0.5)")), Some(Color::Rgba([1.0, 0.0, 0.0, 0.5])));
}

#[test]
fn figma_style_svg_wrapping_group_unwrapped_layers_named_by_id() {
    let art = svg_layers(CHECK, &[]).unwrap();
    assert_eq!(art.width, 120.0);
    assert_eq!(art.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Circle", "Check", "Dot", "Dot_2"]);
    assert!(art.warnings.iter().any(|w| w.contains("clip-path")));
    assert_eq!(art.layers[1].items[0].fill, None, "fill=none inherited from <svg>");
    assert_eq!(art.layers[1].items[0].stroke, Some(Paint::Solid([1.0, 1.0, 1.0, 1.0])));
}

#[test]
fn compile_tracks_become_keyframes_on_the_right_layers() {
    let out = compile_lottie(&json!({
        "name": "success", "duration": 900, "fps": 60, "svg": CHECK,
        "tracks": [
            { "layer": "Circle", "prop": "scale", "from": 0, "to": 1, "start": 0, "end": 400, "ease": "back" },
            { "layer": "Check", "prop": "draw", "from": 0, "to": 1, "start": 300, "end": 700, "ease": "out" },
            { "layer": "dot*", "prop": "opacity", "from": 0, "to": 1, "start": 500, "duration": 200, "stagger": 100 },
        ],
    }))
    .unwrap();
    assert!(warnings(&out).iter().all(|w| w.contains("clip-path")));
    let lottie = &out["lottie"];
    assert_eq!(lottie["op"], 54);
    assert_eq!(lottie["layers"].as_array().unwrap().len(), 4);
    // layer order: last SVG element on top = first Lottie layer
    assert_eq!(lottie["layers"][0]["nm"], "Dot_2");
    let scale = &layer(lottie, "Circle")["ks"]["s"];
    assert_eq!(scale["a"], 1);
    assert_eq!(scale["k"][0]["s"], json!([0, 0, 100]));
    assert_eq!(scale["k"][1]["s"], json!([100, 100, 100]));
    assert_eq!(scale["k"][1]["t"], 24);
    assert_eq!(scale["k"][0]["o"]["y"], json!([1.56]), "back ease overshoots");
    // pivot = centre of the art
    assert_eq!(layer(lottie, "Circle")["ks"]["p"]["k"], json!([60, 60, 0]));
    let trim = layer(lottie, "Check")["shapes"][0]["it"].as_array().unwrap().iter().find(|x| x["ty"] == "tm").unwrap();
    assert_eq!(trim["e"]["k"][0]["t"], 18);
    assert_eq!(layer(lottie, "Dot_2")["ks"]["o"]["k"][0]["t"], 36, "stagger: 2nd dot starts 100ms later");
    assert!(out["summary"].as_str().unwrap().contains("Circle: scale"));
}

#[test]
fn spring_is_sampled_into_keys_that_overshoot_and_settle_on_the_target() {
    let out = compile_lottie(&json!({ "duration": 600, "svg": CHECK, "tracks": [{ "layer": "Circle", "prop": "scale", "from": 0, "to": 1, "end": 600, "ease": "spring" }] })).unwrap();
    let keys = layer(&out["lottie"], "Circle")["ks"]["s"]["k"].as_array().unwrap().clone();
    assert!(keys.len() > 5);
    assert!(keys.iter().map(|k| k["s"][0].as_f64().unwrap()).fold(f64::MIN, f64::max) > 100.0, "overshoots");
    assert_eq!(keys.last().unwrap()["s"], json!([100, 100, 100]));
}

#[test]
fn unknown_layer_names_are_reported_with_the_real_ones() {
    let out = compile_lottie(&json!({ "duration": 500, "svg": CHECK, "tracks": [{ "layer": "tick", "prop": "draw", "from": 0, "to": 1 }] })).unwrap();
    assert!(warnings(&out).iter().any(|w| w.contains("no layer named \"tick\"") && w.contains("Circle, Check")));
}

#[test]
fn layers_with_the_same_name_get_suffixes_and_all_reach_a_glob_track() {
    let svg = r#"<svg width="40" height="40"><circle id="Dot" cx="5" cy="5" r="2"/><circle id="Dot" cx="15" cy="5" r="2"/><circle id="Dot" cx="25" cy="5" r="2"/></svg>"#;
    let out = compile_lottie(&json!({ "duration": 400, "svg": svg, "tracks": [{ "layer": "Dot*", "prop": "scale", "from": 0, "to": 1, "ease": "spring" }] })).unwrap();
    let layers = out["lottie"]["layers"].as_array().unwrap();
    let mut names: Vec<&str> = layers.iter().map(|l| l["nm"].as_str().unwrap()).collect();
    names.reverse();
    assert_eq!(names, ["Dot", "Dot_2", "Dot_3"]);
    assert!(layers.iter().all(|l| l["ks"]["s"]["a"] == 1));
    assert!(warnings(&out).is_empty());
}

#[test]
fn gradients_keep_their_stops_in_the_arts_space() {
    let svg = r##"<svg width="100" height="100" viewBox="0 0 100 100"><defs>
<radialGradient id="ball" cx="0.35" cy="0.3" r="0.7"><stop offset="0" stop-color="#93C5FD"/><stop offset="1" stop-color="#1E3A8A"/></radialGradient>
<linearGradient id="glow" x1="0" y1="0" x2="0" y2="100" gradientUnits="userSpaceOnUse"><stop offset="0%" stop-color="#2563EB" stop-opacity="0.6"/><stop offset="100%" stop-color="#2563EB" stop-opacity="0"/></linearGradient></defs>
<circle id="Ball" cx="50" cy="50" r="20" fill="url(#ball)"/><rect id="Glow" width="100" height="100" fill="url(#glow)"/></svg>"##;
    let out = compile_lottie(&json!({ "duration": 300, "svg": svg, "tracks": [] })).unwrap();
    assert!(warnings(&out).is_empty());
    let gf = |name: &str| layer(&out["lottie"], name)["shapes"][0]["it"].as_array().unwrap().iter().find(|x| x["ty"] == "gf").unwrap().clone();
    let ball = gf("Ball");
    assert_eq!(ball["t"], 2);
    assert_eq!(ball["g"]["p"], 2);
    assert_eq!(ball["s"]["k"], json!([-6, -8]), "centre at 35%/30% of the ball's box, relative to its pivot");
    let glow = gf("Glow");
    assert_eq!(glow["t"], 1);
    let k = glow["g"]["k"]["k"].as_array().unwrap();
    assert_eq!(k.len(), 8 + 4, "colour stops then opacity stops");
    assert_eq!(Value::Array(k[8..].to_vec()), json!([0, 0.6, 1, 0]));
}

#[test]
fn inside_a_layer_later_svg_shapes_stay_on_top() {
    let svg = r##"<svg width="40" height="40"><g id="Card"><rect id="Bg" width="40" height="40" fill="#ffffff"/><rect id="Line" x="5" y="5" width="30" height="4" fill="#0000ff"/></g><circle id="Dot" cx="2" cy="2" r="1"/></svg>"##;
    let out = compile_lottie(&json!({ "duration": 100, "svg": svg, "tracks": [] })).unwrap();
    let fills: Vec<Value> = layer(&out["lottie"], "Card")["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["it"].as_array().unwrap().iter().find(|x| x["ty"] == "fl").unwrap()["c"]["k"].clone())
        .collect();
    assert_eq!(Value::Array(fills), json!([[0, 0, 1, 1], [1, 1, 1, 1]]));
}

// from motion-figma.test.mjs: "Lottie hold ease → h:1 keyframe"
#[test]
fn lottie_hold_ease_is_a_hold_keyframe() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect id="Box" width="10" height="10" fill="#f00"/></svg>"##;
    let out = compile_lottie(&json!({ "svg": svg, "duration": 1000, "tracks": [{ "layer": "Box", "prop": "opacity", "keys": [[0, 0, "hold"], [500, 1]] }] })).unwrap();
    assert_eq!(out["lottie"]["layers"][0]["ks"]["o"]["k"][0]["h"], 1);
}
