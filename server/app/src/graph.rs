//! Port of graph.mjs: turns design_map (+ check_flows) output into { nodes, links } for the force-graph viewer.

use std::collections::HashSet;

use serde_json::{json, Map, Value};

fn arr(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

fn key(v: &Value) -> String {
    crate::jsjson::js_string(Some(v))
}

pub fn to_graph_data(map: &Value, flows: Option<&Value>) -> Value {
    let mut nodes: Vec<Value> = vec![];
    let mut links: Vec<Value> = vec![];
    let pages = arr(map.get("pages"));
    let flow_starts: HashSet<String> = pages
        .iter()
        .flat_map(|p| arr(p.get("flows")).iter().map(|f| f.get("nodeId").map(key).unwrap_or_default()))
        .collect();
    let ids_of = |name: &str| -> HashSet<String> { arr(flows.and_then(|f| f.get(name))).iter().map(|d| d.get("id").map(key).unwrap_or_default()).collect() };
    let dead_ends = ids_of("deadEnds");
    let orphans = ids_of("orphans");
    let mut screen_ids: HashSet<String> = HashSet::new();

    for page in pages {
        for s in arr(page.get("screens")) {
            let id = s.get("id").cloned().unwrap_or(Value::Null);
            let id_key = key(&id);
            screen_ids.insert(id_key.clone());
            let mut node = Map::new();
            node.insert("id".into(), id);
            node.insert("name".into(), s.get("name").cloned().unwrap_or(Value::Null));
            node.insert("type".into(), "screen".into());
            node.insert("page".into(), page.get("name").cloned().unwrap_or(Value::Null));
            node.insert("size".into(), s.get("size").cloned().unwrap_or(Value::Null));
            node.insert("flowStart".into(), Value::Bool(flow_starts.contains(&id_key)));
            if dead_ends.contains(&id_key) {
                node.insert("issue".into(), "dead-end".into());
            } else if orphans.contains(&id_key) {
                node.insert("issue".into(), "orphan".into());
            }
            nodes.push(Value::Object(node));
        }
    }

    let mut components: HashSet<String> = HashSet::new();
    for c in arr(map.get("components")) {
        let name = c.get(0).cloned().unwrap_or(Value::Null);
        components.insert(key(&name));
        nodes.push(json!({
            "id": format!("c:{}", key(&name)),
            "name": name,
            "type": "component",
            "count": c.get(1).cloned().unwrap_or(Value::Null),
            "screens": c.get(2).cloned().unwrap_or(Value::Null),
        }));
    }
    for name in arr(map.get("unusedComponents")) {
        if !components.contains(&key(name)) {
            nodes.push(json!({"id": format!("c:{}", key(name)), "name": name, "type": "component", "count": 0, "screens": 0, "issue": "unused"}));
        }
    }

    for page in pages {
        for s in arr(page.get("screens")) {
            let id = s.get("id").cloned().unwrap_or(Value::Null);
            for c in arr(s.get("components")) {
                let name = c.get(0).map(key).unwrap_or_else(|| "undefined".into());
                links.push(json!({"source": id, "target": format!("c:{name}"), "type": "uses", "value": c.get(1).cloned().unwrap_or(Value::Null)}));
            }
            for to in arr(s.get("toIds")) {
                if screen_ids.contains(&key(to)) {
                    links.push(json!({"source": id, "target": to, "type": "navigates"}));
                }
            }
        }
    }

    if let Some(Value::Object(sets)) = map.get("breakpoints") {
        for (set_name, bps) in sets {
            let ids: Vec<&Value> = match bps {
                Value::Object(o) => o.values().filter(|id| screen_ids.contains(&key(id))).collect(),
                _ => vec![],
            };
            for i in 1..ids.len() {
                links.push(json!({"source": ids[i - 1], "target": ids[i], "type": "breakpoint", "set": set_name}));
            }
        }
    }

    json!({
        "file": map.get("file").cloned().unwrap_or(Value::Null),
        "pages": pages.iter().map(|p| json!({"id": p.get("id").cloned().unwrap_or(Value::Null), "name": p.get("name").cloned().unwrap_or(Value::Null)})).collect::<Vec<_>>(),
        "flowsChecked": flows.is_some(),
        "nodes": nodes,
        "links": links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsjson::stringify;

    #[test]
    fn screens_components_links_and_issues() {
        let map = json!({
            "file": "utils",
            "pages": [{"id": "0:1", "name": "App", "flows": [{"name": "Main", "nodeId": "1:1"}], "screens": [
                {"id": "1:1", "name": "Home", "size": "375×812", "components": [["Button", 2]], "toIds": ["1:2", "9:9"]},
                {"id": "1:2", "name": "Detail", "size": "375×812", "components": [], "toIds": []}
            ]}],
            "components": [["Button", 2, 1]],
            "unusedComponents": ["Card", "Button"],
            "breakpoints": {"Home": {"base": "1:1", "md": "1:2"}}
        });
        let flows = json!({"deadEnds": [{"id": "1:2"}], "orphans": []});
        let got = stringify(&to_graph_data(&map, Some(&flows)));
        assert_eq!(
            got,
            r#"{"file":"utils","pages":[{"id":"0:1","name":"App"}],"flowsChecked":true,"nodes":[{"id":"1:1","name":"Home","type":"screen","page":"App","size":"375×812","flowStart":true},{"id":"1:2","name":"Detail","type":"screen","page":"App","size":"375×812","flowStart":false,"issue":"dead-end"},{"id":"c:Button","name":"Button","type":"component","count":2,"screens":1},{"id":"c:Card","name":"Card","type":"component","count":0,"screens":0,"issue":"unused"}],"links":[{"source":"1:1","target":"c:Button","type":"uses","value":2},{"source":"1:1","target":"1:2","type":"navigates"},{"source":"1:1","target":"1:2","type":"breakpoint","set":"Home"}]}"#
        );
    }
}
