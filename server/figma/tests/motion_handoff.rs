//! Port of legacy-node/test/motion-handoff.test.mjs.
use regex::Regex;
use serde_json::{json, Value};
use wridge_figma::motion_handoff::{check_text, classify, lint, motion_context, to_css, to_framer, tokens, Row};

fn vec_hint() -> Value {
    json!({ "text": false, "instances": 0, "autoLayout": false, "vectorOnly": true, "image": false, "component": null })
}
fn ui_hint() -> Value {
    let mut h = vec_hint();
    h["vectorOnly"] = json!(false);
    h["autoLayout"] = json!(true);
    h["text"] = json!(true);
    h
}
fn with(mut h: Value, k: &str, v: Value) -> Value {
    h[k] = v;
    h
}
fn key(t: f64, v: Value, ease: Option<&str>) -> Value {
    let mut k = json!({ "t": t, "v": v });
    if let Some(e) = ease {
        k["ease"] = json!(e);
    }
    k
}
fn field(name: &str, keys: Vec<Value>, op: &str, base: Value) -> Value {
    json!({ "field": name, "base": base, "tracks": [{ "op": op, "keys": keys }] })
}
fn f(name: &str, keys: Vec<Value>) -> Value {
    field(name, keys, "SET", json!(1))
}

// an illustration: ring scales (simple), check draws (path) → the whole art is one Lottie
fn art() -> Value {
    json!({ "root": { "id": "A", "name": "Success" }, "timelines": [{ "duration": 1.2 }], "animated": [
        { "id": "A1", "name": "Ring", "parentId": "A", "nth": 1, "hint": vec_hint(), "styles": [], "tracks": [f("SCALE_X", vec![key(0.0, json!(0), Some("BOUNCY")), key(0.5, json!(1), None)]), f("SCALE_Y", vec![key(0.0, json!(0), Some("BOUNCY")), key(0.5, json!(1), None)])] },
        { "id": "A2", "name": "Check", "parentId": "A", "nth": 1, "hint": vec_hint(), "styles": [], "tracks": [f("PATH_TRIM_END", vec![key(0.3, json!(0), Some("cubic(0.2,0,0,1)")), key(0.7, json!(1), None)])] },
    ] })
}
// a screen: title + list card slide/fade in, a Lottie/ component, a slow card that animates width
fn screen() -> Value {
    json!({ "root": { "id": "S", "name": "Home" }, "timelines": [{ "duration": 2 }], "animated": [
        { "id": "S1", "name": "Title", "parentId": "S", "nth": 1, "hint": ui_hint(), "styles": [], "tracks": [f("OPACITY", vec![key(0.0, json!(0), Some("EASE_OUT")), key(0.2, json!(1), None)]), field("TRANSLATION_Y", vec![key(0.0, json!(16), Some("EASE_OUT")), key(0.2, json!(0), None)], "SET", json!(24))] },
        { "id": "S2", "name": "Card", "parentId": "S", "nth": 1, "hint": with(ui_hint(), "component", json!("Card")), "styles": [], "tracks": [f("OPACITY", vec![key(0.1, json!(0), Some("EASE_OUT")), key(0.4, json!(1), None)]), field("WIDTH", vec![key(0.1, json!(200), Some("EASE_OUT")), key(0.8, json!(320), None)], "SET", json!(320))] },
        { "id": "S3", "name": "Confetti", "parentId": "S", "nth": 1, "hint": with(vec_hint(), "component", json!("Lottie/Confetti")), "styles": [], "tracks": [f("OPACITY", vec![key(0.0, json!(0), None), key(1.5, json!(1), None)])] },
    ] })
}

fn rows(r: &Value, context: &str) -> Vec<Row> {
    classify(r, &mut vec![], context).unwrap()
}
fn to(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter().map(|r| (r.name.as_ref().unwrap().as_str().unwrap().to_string(), r.to.clone())).collect()
}
fn m(re: &str, s: &str) -> bool {
    Regex::new(re).unwrap().is_match(s)
}

#[test]
fn an_illustration_goes_to_lottie_as_one_piece() {
    let r = art();
    let rs = rows(&r, "ui");
    assert_eq!(to(&rs), [("Ring".into(), "lottie".into()), ("Check".into(), "lottie".into())]);
    assert!(rs[0].why.contains("same illustration"));
    assert!(check_text(&r, &rs, &lint(&rs, &r, "ui"), "ui").contains("motion_to_lottie on Success (A)"));
}

#[test]
fn a_screen_ui_to_code_lottie_components_to_lottie_lint() {
    let r = screen();
    let rs = rows(&r, "ui");
    assert_eq!(to(&rs), [("Title".into(), "code".into()), ("Card".into(), "code".into()), ("Confetti".into(), "lottie".into())]);
    let l = lint(&rs, &r, "ui").join("\n");
    assert!(l.contains("Card: 700ms of UI motion"));
    assert!(l.contains("width/height"));
    assert!(l.contains("mixes Lottie art and UI layers"));
}

#[test]
fn tokens_css_and_framer_motion() {
    let r = screen();
    let rs = rows(&r, "ui");
    let tk = tokens(&rs);
    assert_eq!(tk.duration, vec![("fast".to_string(), 200.0), ("base".to_string(), 700.0)]);
    assert_eq!(tk.easing.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["out"]);
    assert!(tk.css.contains("--motion-duration-fast: 200ms;"));
    let css = to_css(&rs, &tk, &mut vec![]);
    assert!(css.contains("@keyframes title {"));
    assert!(css.contains("0% { transform: translate(0px, 16px); opacity: 0; animation-timing-function: var(--motion-easing-out); }"));
    assert!(css.contains(".title {\n  animation: title var(--motion-duration-fast) both;"));
    assert!(css.contains("animation: card var(--motion-duration-base) 100ms both;"));
    assert!(css.contains("prefers-reduced-motion"));
    assert!(!css.contains("confetti"));
    let jsx = to_framer(&rs, &tk);
    assert!(jsx.contains("opacity: [0, 1], y: [16, 0]"));
    assert!(m(r#"opacity: \{ duration: 0.3, delay: 0.1, times: \[0, 1\], ease: \["easeOut"\] \}"#, &jsx));
}

#[test]
fn a_list_appearing_one_by_one_is_one_animation_with_a_delay_by_index() {
    let card = |id: &str, nth: i64, t: f64| {
        json!({ "id": id, "name": "Card", "parentId": "H", "nth": nth, "hint": ui_hint(), "styles": [], "tracks": [
            f("OPACITY", vec![key(t, json!(0), Some("cubic(0.2,0,0,1)")), key(t + 0.3, json!(1), None)]),
            field("TRANSLATION_Y", vec![key(t, json!(24), Some("cubic(0.2,0,0,1)")), key(t + 0.3, json!(0), None)], "SET", json!(76 + nth * 72)),
        ] })
    };
    let home = json!({ "root": { "id": "H", "name": "Home" }, "timelines": [{ "duration": 0.8 }], "animated": [card("H1", 1, 0.1), card("H2", 2, 0.16), card("H3", 3, 0.22)] });
    let rs = rows(&home, "ui");
    let tk = tokens(&rs);
    assert_eq!(tk.easing.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["standard"]);
    let css = to_css(&rs, &tk, &mut vec![]);
    assert_eq!(css.matches("@keyframes").count(), 1);
    assert!(css.contains("3 items, 60ms apart"));
    assert!(css.contains("animation: card var(--motion-duration-base) calc(100ms + var(--i, 0) * 60ms) both;"));
    assert!(css.contains("0% { transform: translate(0px, 24px)"));
    let jsx = to_framer(&rs, &tk);
    assert!(jsx.contains("{items.map((item, i) => ("));
    assert!(jsx.contains("delay: 0.1 + i * 0.06"));
}

#[test]
fn ai_smell_identical_layers_are_flagged_a_led_composition_is_not() {
    let item = |id: &str, t: f64, dist: f64, dur: f64, ease: &str| {
        json!({ "id": id, "name": format!("Item {id}"), "parentId": "P", "nth": 1, "hint": ui_hint(), "styles": [], "tracks": [
            f("OPACITY", vec![key(t, json!(0), Some(ease)), key(t + dur, json!(1), None)]),
            field("TRANSLATION_Y", vec![key(t, json!(dist), Some(ease)), key(t + dur, json!(0), None)], "SET", json!(100)),
        ] })
    };
    let read = |animated: Vec<Value>| json!({ "root": { "id": "P", "name": "Page" }, "timelines": [{ "duration": 2 }], "animated": animated });
    let flat = read(["a", "b", "c", "d"].iter().map(|id| item(id, 0.0, 20.0, 0.3, "EASE_IN")).collect());
    let smell = lint(&rows(&flat, "ui"), &flat, "ui").into_iter().filter(|l| l.starts_with("AI smell")).collect::<Vec<_>>().join("\n");
    assert!(smell.contains("Item a, Item b, Item c, Item d appear with in"));
    assert!(smell.contains("all 4 layers last 300ms"));
    assert!(smell.contains("one easing"));
    assert!(smell.contains("everything starts at once"));
    assert!(smell.contains("fades in and moves from [0,20]"));
    let led = read(vec![item("a", 0.0, 32.0, 0.42, "cubic(0.16,1,0.3,1)"), item("b", 0.12, 12.0, 0.26, "EASE_OUT"), item("c", 0.16, 12.0, 0.26, "EASE_OUT")]);
    assert!(lint(&rows(&led, "ui"), &led, "ui").iter().all(|l| !l.starts_with("AI smell")));
}

#[test]
fn landing_context_art_is_part_of_the_scroll_scene() {
    assert_eq!(motion_context(Some(&json!("Eyewear Atelier landing · Storyboard · Draft"))), "landing");
    assert_eq!(motion_context(Some(&json!("Checkout · Prototype · Draft"))), "ui");
    let a = art();
    assert!(rows(&a, "ui").iter().all(|x| x.to == "lottie"));
    assert!(rows(&a, "landing").iter().all(|x| x.to == "code" && x.why.contains("scroll scene")));
    let slow = json!({ "root": { "id": "L", "name": "Hero" }, "timelines": [{ "duration": 1.5 }], "animated": [
        { "id": "L1", "name": "Glasses", "parentId": "L", "nth": 1, "hint": vec_hint(), "styles": [], "tracks": [f("OPACITY", vec![key(0.0, json!(0), Some("EASE_OUT")), key(0.9, json!(1), None)])] },
    ] });
    assert!(lint(&rows(&slow, "ui"), &slow, "ui").join("\n").contains("feels slow"));
    let l = lint(&rows(&slow, "landing"), &slow, "landing").join("\n");
    assert!(!m("feels slow|usable within", &l));
}
