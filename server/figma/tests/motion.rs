//! Port of legacy-node/test/motion.test.mjs (temp folders only).
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use wridge_figma::lottie::compile_lottie;
use wridge_figma::motion::{content_hash, list_lottie, mark_sent_to_figma, read_lottie};

const SVG: &str = r##"<svg width="40" height="40" viewBox="0 0 40 40"><circle id="Dot" cx="20" cy="20" r="10" fill="#2563EB"/></svg>"##;

fn project() -> tempfile::TempDir {
    let root = tempfile::Builder::new().prefix("wara-motion-").tempdir().unwrap();
    let r = root.path();
    fs::create_dir_all(r.join("assets/lottie")).unwrap();
    fs::create_dir_all(r.join("node_modules/pkg")).unwrap();
    let mut lottie = compile_lottie(&json!({ "name": "pop", "duration": 500, "svg": SVG, "tracks": [{ "layer": "Dot", "prop": "scale", "from": 0, "to": 1 }] })).unwrap()["lottie"].clone();
    lottie["meta"] = json!({ "g": "wara-bridge", "wara": { "spec": { "duration": 500, "tracks": [] } } });
    let text = wridge_figma::js::stringify(&lottie);
    fs::write(r.join("assets/lottie/pop.json"), &text).unwrap();
    fs::write(r.join("node_modules/pkg/anim.json"), &text).unwrap(); // skipped folder
    fs::write(r.join("assets/data.json"), json!({ "layers": [], "fr": "x", "other": "not lottie".repeat(30) }).to_string()).unwrap();
    fs::write(r.join("package.json"), "{}").unwrap();
    root
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn finds_lottie_files_by_content_skips_node_modules_and_other_json() {
    let root = project();
    let files = list_lottie(&s(root.path())).unwrap();
    assert_eq!(files.iter().map(|f| f["path"].as_str().unwrap()).collect::<Vec<_>>(), ["assets/lottie/pop.json"]);
    assert_eq!(files[0]["wara"], true);
    assert_eq!(files[0]["durationMs"], 500);
    assert_eq!(files[0]["figma"], Value::Null);
}

#[test]
fn paths_outside_the_project_are_refused() {
    let root = project();
    assert!(read_lottie(&s(root.path()), "../secret.json").unwrap_err().to_string().contains("inside the project"));
    assert!(read_lottie(&s(root.path()), "assets/data.json").unwrap_err().to_string().contains("not a Lottie"));
}

#[test]
fn sent_to_figma_remembered_and_stale_once_the_animation_changes() {
    let root = project();
    let dir = s(root.path());
    let sent = mark_sent_to_figma(&dir, "assets/lottie/pop.json", &json!({ "componentId": "1:2", "file": "wara-bridge" })).unwrap();
    assert_eq!((sent["figma"]["componentId"].clone(), sent["figma"]["stale"].clone()), (json!("1:2"), json!(false)));
    let p = root.path().join("assets/lottie/pop.json");
    let mut j: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(j["meta"]["wara"]["spec"]["duration"], 500, "recipe kept");
    j["op"] = json!(j["op"].as_i64().unwrap() + 6); // the animation changed (e.g. create_lottie again)
    fs::write(&p, wridge_figma::js::stringify(&j)).unwrap();
    assert_eq!(list_lottie(&dir).unwrap()[0]["figma"]["stale"], true);
    assert_ne!(json!(content_hash(&j)), j["meta"]["wara"]["figma"]["hash"]);
}
