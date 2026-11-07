//! Port of legacy-node/test/motion-figma.test.mjs.
use serde_json::{json, Value};
use std::f64::consts::PI;
use wridge_figma::lottie::compile_lottie;
use wridge_figma::motion_figma::{motion_to_spec, presets_to_items, to_easing, tracks_to_items};

fn items(v: Value) -> Vec<Value> {
    tracks_to_items(v.as_array().unwrap()).unwrap()
}
fn has(w: &Value, f: impl Fn(&str) -> bool) -> bool {
    w.as_array().unwrap().iter().any(|x| f(x.as_str().unwrap()))
}

#[test]
fn from_to_track_ms_to_s_ease_on_the_start_key_rotate_in_radians() {
    let it = &items(json!([{ "layer": "Box", "prop": "rotate", "from": 0, "to": 90, "start": 200, "end": 1000, "ease": "back" }]))[0];
    assert_eq!(it["field"], json!({ "type": "PROPERTY", "name": "ROTATION" }));
    assert_eq!(it["keys"].as_array().unwrap().iter().map(|k| k["t"].clone()).collect::<Vec<_>>(), [json!(0.2), json!(1)]);
    assert_eq!(it["keys"][1]["value"]["value"].as_f64().unwrap(), PI / 2.0);
    assert_eq!(it["keys"][0]["easing"], json!({ "type": "EASE_OUT_BACK" }));
    assert!(it["keys"][1].get("easing").is_none());
}

#[test]
fn move_and_scale_split_into_x_y_fields_fill_becomes_a_colour() {
    let its = items(json!([{ "prop": "move", "from": [0, 20], "to": [0, 0] }, { "prop": "scale", "from": 0.5 }, { "prop": "fill", "from": "#ff0000", "to": "#00ff0080" }]));
    let names: Vec<&str> = its.iter().map(|i| i["field"].get("name").or(i["field"].get("collection")).unwrap().as_str().unwrap()).collect();
    assert_eq!(names, ["TRANSLATION_X", "TRANSLATION_Y", "SCALE_X", "SCALE_Y", "fills"]);
    assert_eq!(its[1]["keys"][0]["value"]["value"], 20);
    assert_eq!(its[3]["keys"][1]["value"]["value"], 1); // scale rests at 1
    assert_eq!(its[4]["keys"][1]["value"]["value"]["a"].as_f64().unwrap(), 128.0 / 255.0);
}

#[test]
fn keys_form_each_ease_leaves_its_key() {
    let it = &items(json!([{ "prop": "opacity", "keys": [[0, 0, "hold"], [300, 1], [600, 0.5]], "ease": "out" }]))[0];
    let types: Vec<Option<&str>> = it["keys"].as_array().unwrap().iter().map(|k| k.get("easing").map(|e| e["type"].as_str().unwrap())).collect();
    assert_eq!(types, [Some("HOLD"), Some("EASE_OUT"), None]);
}

#[test]
fn easing_names_cubic_and_errors() {
    assert_eq!(to_easing(Some(&json!("cubic(0.2,0,0,1)"))).unwrap(), to_easing(Some(&json!("standard"))).unwrap());
    assert!(to_easing(Some(&json!("wobble"))).unwrap_err().to_string().contains("Unknown ease"));
    assert!(tracks_to_items(&[json!({ "prop": "skew" })]).unwrap_err().to_string().contains("prop \"skew\""));
}

#[test]
fn presets_matched_by_short_name_delay_duration_in_seconds() {
    let it = &presets_to_items(&[json!({ "layer": "Box", "style": "opacity", "delay": 500, "duration": 800, "stagger": 100 })], &[json!({ "styleId": "Opacity", "name": "motion.preset_name.opacity" })]).unwrap()[0];
    assert_eq!(it["styleId"], "Opacity");
    assert_eq!(it["config"]["duration"], 0.8);
    assert_eq!(it["config"]["props"]["delay"], 0.5);
    assert_eq!(it["stagger"], 0.1);
    assert!(presets_to_items(&[json!({ "style": "Wiggle" })], &[]).unwrap_err().to_string().contains("no animation style"));
}

#[test]
fn motion_to_spec_figma_tracks_to_create_lottie_spec() {
    let read = json!({
        "root": { "id": "1:1", "name": "Card" }, "timelines": [{ "id": "t", "duration": 1.5 }],
        "animated": [
            { "id": "1:1", "name": "Card", "parentId": "0:1", "nth": 1, "tracks": [{ "field": "OPACITY", "base": 1, "tracks": [{ "op": "SET", "keys": [{ "t": 0, "v": 0 }, { "t": 1, "v": 1 }] }] }], "styles": [] },
            { "id": "1:2", "name": "Dot", "parentId": "1:1", "nth": 2, "tracks": [
                { "field": "ROTATION", "base": 0, "tracks": [{ "op": "SET", "keys": [{ "t": 0.2, "v": 0, "ease": "EASE_OUT_BACK" }, { "t": 1, "v": PI / 2.0, "ease": "cubic(0.5,0,0.5,1)" }] }] },
                { "field": "OPACITY", "base": 1, "tracks": [{ "op": "SCALE", "keys": [{ "t": 0, "v": 0, "ease": "HOLD" }, { "t": 0.8, "v": 1 }] }] },
                { "field": "TRANSLATION_Y", "base": 0, "tracks": [{ "op": "SET", "keys": [{ "t": 0, "v": 20, "ease": "GENTLE" }, { "t": 0.5, "v": 0 }] }] },
            ], "styles": [] },
            { "id": "1:3", "name": "Inner", "parentId": "1:2", "nth": 1, "tracks": [{ "field": "OPACITY", "base": 1, "tracks": [{ "op": "SET", "keys": [{ "t": 0, "v": 0 }, { "t": 1, "v": 1 }] }] }], "styles": [] },
        ],
    });
    let spec = motion_to_spec(&read).unwrap();
    assert_eq!(spec["duration"], 1500);
    let tracks = spec["tracks"].as_array().unwrap();
    let find = |p: &str| tracks.iter().find(|t| t["prop"] == p).unwrap();
    assert_eq!(find("rotate")["layer"], "Dot_2");
    assert_eq!(find("rotate")["keys"], json!([[200, 0, "back"], [1000, 90, "cubic(0.5,0,0.5,1)"]]));
    assert_eq!(find("opacity")["keys"], json!([[0, 0, "hold"], [800, 1]]));
    assert_eq!(find("move")["keys"], json!([[0, [0, 20], "decelerate"], [500, [0, 0]]]));
    assert!(has(&spec["warnings"], |w| w.contains("frame itself")));
    assert!(has(&spec["warnings"], |w| w.contains("Inner") && w.contains("nested layer")));
    assert!(has(&spec["warnings"], |w| w.contains("GENTLE")));
}

#[test]
fn nested_layers_open_their_groups_in_the_svg() {
    let read = json!({ "root": { "id": "F", "name": "Scene" }, "timelines": [{ "duration": 1 }], "animated": [
        { "id": "F2", "name": "Lens L", "parentId": "G", "nth": 1, "path": ["Glasses"], "tracks": [{ "field": "TRANSLATION_Y", "base": 10, "tracks": [{ "op": "SET", "keys": [{ "t": 0, "v": 50, "ease": "EASE_OUT" }, { "t": 0.5, "v": 0 }] }] }], "styles": [] },
    ] });
    let spec = motion_to_spec(&read).unwrap();
    assert_eq!(spec["split"], json!(["Glasses"]));
    assert_eq!(spec["tracks"][0]["keys"], json!([[0, [0, 50], "out"], [500, [0, 0]]]));
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60"><g id="Glasses"><rect id="Lens L" x="0" y="10" width="40" height="30" fill="#ccc"/><rect id="Lens R" x="60" y="10" width="40" height="30" fill="#ccc"/></g><rect id="Shadow" x="0" y="50" width="100" height="4" fill="#000"/></svg>"##;
    let out = compile_lottie(&json!({ "svg": svg, "duration": 1000, "tracks": spec["tracks"], "split": spec["split"] })).unwrap();
    let mut names: Vec<&str> = out["lottie"]["layers"].as_array().unwrap().iter().map(|l| l["nm"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["Lens L", "Lens R", "Shadow"]);
    assert!(out["summary"].as_str().unwrap().contains("Lens L: move"));
}

#[test]
fn plugin_never_reads_an_instances_main_component_synchronously() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../figma-plugin/code.ts")).unwrap();
    let re = regex::Regex::new(r"\.mainComponent\b").unwrap();
    let sync: Vec<(usize, &str)> = src.lines().enumerate().map(|(i, l)| (i + 1, l)).filter(|(_, l)| re.is_match(l)).collect();
    assert!(sync.is_empty(), "use await node.getMainComponentAsync() instead: {sync:?}");
}

#[test]
fn move_goes_to_figma_as_offsets_and_reads_back_the_same() {
    let its = items(json!([{ "layer": "Fold1", "prop": "move", "keys": [[0, [0, 0]], [1300, [-283, 0]]] }]));
    let got: Vec<Value> = its.iter().map(|i| json!([i["field"]["name"], i.get("relative"), i["keys"].as_array().unwrap().iter().map(|k| k["value"]["value"].clone()).collect::<Vec<_>>()])).collect();
    assert_eq!(Value::Array(got), json!([["TRANSLATION_X", null, [0, -283]], ["TRANSLATION_Y", null, [0, 0]]]));
    let read = json!({ "root": { "id": "F", "name": "Folds" }, "timelines": [{ "duration": 2 }], "animated": [
        { "id": "F1", "name": "Fold1", "parentId": "F", "nth": 1, "path": [], "tracks": [
            { "field": "TRANSLATION_X", "base": 480, "tracks": [{ "op": "SET", "keys": [{ "t": 0, "v": 0 }, { "t": 1.3, "v": -283 }] }] },
        ], "styles": [] },
    ] });
    assert_eq!(motion_to_spec(&read).unwrap()["tracks"][0]["keys"], json!([[0, [0, 0]], [1300, [-283, 0]]]));
}
