//! The codegen tests of legacy-node/test/bridge.test.mjs, plus the component map round trip.
use serde_json::json;
use wridge_figma::codegen::{format_tokens, read_component_map, resolve_entry, write_component_map};

#[test]
fn class_mappings_base_classes_plus_the_classes_of_each_variant_value() {
    let entry = json!({ "element": "button", "className": "btn", "variants": { "Type=Primary": "btn-p", "Type=Danger": "btn-d", "Size=Small": "btn-sm" } });
    assert_eq!(resolve_entry(&entry, Some(&json!("Type=Danger, State=Hover, Size=Small"))), json!({ "element": "button", "className": "btn btn-d btn-sm" }));
    assert_eq!(resolve_entry(&json!({ "import": "@/ui/button", "component": "Button" }), Some(&json!("Type=Primary")))["component"], "Button");
}

#[test]
fn tailwind_tokens_a_text_style_or_shadow_named_like_a_color_is_renamed() {
    let data = json!({
        "file": "t",
        "collections": [{ "name": "c", "defaultMode": "Light" }],
        "variables": [{ "token": "color-tile", "type": "COLOR", "collection": "c", "values": { "Light": "#ffffff" } }, { "token": "color-card", "type": "COLOR", "collection": "c", "values": { "Light": "#eeeeee" } }],
        "paintStyles": [],
        "textStyles": [{ "token": "text-tile", "family": "Kanit", "style": "Bold", "size": 24, "lineHeight": "130%", "letterSpacing": "0px" }, { "token": "text-body", "family": "Kanit", "style": "Regular", "size": 16, "lineHeight": "150%", "letterSpacing": "0px" }],
        "effectStyles": [{ "token": "shadow-card", "shadows": [{ "x": 0, "y": 2, "blur": 4, "spread": 0, "color": "#0000001a" }] }],
    });
    let tw = format_tokens(&data, "tailwind");
    assert_eq!(tw["map"]["text-tile"], "--text-type-tile");
    assert_eq!(tw["map"]["text-body"], "--text-body");
    assert_eq!(tw["map"]["shadow-card"], "--shadow-depth-card");
    assert!(tw["content"].as_str().unwrap().contains("Renamed to avoid clashing"));
    assert_eq!(format_tokens(&data, "css")["map"]["shadow-card"], "--shadow-card");
}

#[test]
fn component_map_round_trip_sorted_keys() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_string_lossy().into_owned();
    assert_eq!(read_component_map(&d).unwrap(), json!({}));
    let file = write_component_map(&d, &json!({ "Zeta": { "component": "Z" }, "alpha": { "component": "A" } })).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert_eq!(text, "{\n  \"alpha\": {\n    \"component\": \"A\"\n  },\n  \"Zeta\": {\n    \"component\": \"Z\"\n  }\n}\n");
    assert_eq!(read_component_map(&d).unwrap()["Zeta"]["component"], "Z");
    assert!(read_component_map("relative/dir").unwrap_err().to_string().contains("must be an absolute path"));
}
