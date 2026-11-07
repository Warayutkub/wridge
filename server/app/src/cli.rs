//! Port of wara.mjs: control Figma through the running server from a terminal.

use serde_json::{json, Map, Value};

use crate::jsjson::{js_string, stringify, truthy};

const USAGE: &str = r#"wridge — control Figma through the wridge server

  wridge guide                    how to use wridge (read this first)
  wridge skills                   list skills (design / coding methods)
  wridge skill <name>             print one skill, e.g. skill ui-visual
  wridge status                   server + plugin status
  wridge list                     all tools and their parameters
  wridge <tool> key=value ...     run a tool (values are parsed as JSON when possible)
  wridge <tool> '{"json":true}'   run a tool with a JSON object
  wridge <tool> @args.json        run a tool with arguments from a JSON file (best for run_batch)
  wridge code-graph …             ask a code project's graph (works without the server; see code-graph help)
  wridge memory overview|find <topic> [--dir D]   project memory: open tasks + decisions (no server needed)

examples
  wridge get_selection depth=2
  wridge set_image_fill nodeId=12:34 source=C:/Users/me/pic.jpg scaleMode=FIT
  wridge copy_fills fromNodeId=1:2 'toNodeIds=["3:4","5:6"]'
  wridge export_node_image nodeId=12:34        (saves a PNG and prints its path)"#;

fn base() -> String {
    format!("http://127.0.0.1:{}", std::env::var("WARA_HTTP_PORT").unwrap_or_else(|_| "3156".into()))
}

pub fn parse_args(rest: &[String]) -> anyhow::Result<Value> {
    if rest.is_empty() {
        return Ok(json!({}));
    }
    if let Some(file) = rest[0].strip_prefix('@') {
        let text = String::from_utf8_lossy(&crate::util::read_file(std::path::Path::new(file))?).into_owned();
        return crate::jsjson::parse(&text).map_err(|m| anyhow::anyhow!(m));
    }
    if rest[0].trim_start().starts_with('{') {
        return crate::jsjson::parse(&rest.join(" ")).map_err(|m| anyhow::anyhow!(m));
    }
    let mut args = Map::new();
    for pair in rest {
        let i = pair.find('=').filter(|i| *i >= 1);
        let Some(i) = i else { anyhow::bail!("Expected key=value, got \"{pair}\"") };
        let raw = &pair[i + 1..];
        // not JSON — keep as a plain string (node ids like 12:34, paths, text)
        let value = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        args.insert(pair[..i].to_string(), value);
    }
    Ok(Value::Object(args))
}

async fn request(path: &str, body: Option<&Value>) -> Result<reqwest::Response, i32> {
    let client = reqwest::Client::new();
    let url = format!("{}{path}", base());
    let res = match body {
        None => client.get(&url).send().await,
        Some(b) => {
            let mut post = client.post(&url).header("Content-Type", "application/json");
            // WRIDGE_SESSION names this caller's session (its own Figma file and file check; session.rs)
            if let Some(s) = crate::session::parse(std::env::var("WRIDGE_SESSION").ok().as_deref()) {
                post = post.header(crate::session::HEADER, &*s);
            }
            post.body(stringify(b)).send().await
        }
    };
    res.map_err(|_| {
        eprintln!(
            "wridge server is not running on {}.\nAsk the user to start it (open the wridge app and press Start server).",
            base()
        );
        2
    })
}

async fn call(path: &str, body: Option<&Value>) -> Result<Value, i32> {
    let res = request(path, body).await?;
    let text = res.text().await.map_err(|_| 1)?;
    crate::jsjson::parse(&text).map_err(|m| {
        eprintln!("SyntaxError: {m}");
        1
    })
}

/// All of stdin as text (`memory brief --stdin`).
fn read_stdin() -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `code-graph` / `memory`: wridge-graph's CLI in this process (no server). Prints its text (when there is
/// any); an error ends the program with code 1, as an uncaught error ends `node wara.mjs`.
async fn local(run: impl FnOnce(&str) -> anyhow::Result<String> + Send + 'static) -> i32 {
    crate::paths::Paths::discover().use_dart_dir();
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    match tokio::task::spawn_blocking(move || run(&cwd)).await {
        Ok(Ok(text)) => {
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Ok(Err(err)) => {
            eprintln!("Error: {err}");
            1
        }
        Err(err) => {
            eprintln!("Error: {err}");
            1
        }
    }
}

/// Runs the CLI and returns the exit code.
pub async fn main(argv: Vec<String>) -> i32 {
    match run(argv).await {
        Ok(code) => code,
        Err(code) => code,
    }
}

async fn run(argv: Vec<String>) -> Result<i32, i32> {
    let command = argv.first().cloned();
    let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
    match command.as_deref() {
        None | Some("help") | Some("--help") => println!("{USAGE}"),
        // Local only: reads the project's .wara/code-graph.db, so it works while the server is off.
        Some("code-graph") => return Ok(local(move |cwd| wridge_graph::cli::code_graph_cli(&rest, cwd)).await),
        // Local only. "memory brief --stdin" is the Claude Code SessionStart hook: reads the hook JSON (cwd)
        // from stdin and prints the project's open tasks + decision titles — or nothing outside a project.
        Some("memory") => return Ok(local(move |cwd| wridge_graph::cli::memory_cli(&rest, cwd, read_stdin)).await),
        Some("guide") => {
            let res = request("/guide", None).await?;
            println!("{}", res.text().await.unwrap_or_default());
        }
        Some("skills") | Some("skill") => {
            let args = match rest.first() {
                Some(n) => json!({"name": n}),
                None => json!({}),
            };
            let result = call("/api/tool", Some(&json!({"name": "get_skill", "args": args}))).await?;
            let text = result.get("content").and_then(|c| c.get(0)).and_then(|c| c.get("text"));
            println!("{}", text.map(|t| js_string(Some(t))).unwrap_or_default());
            if truthy(result.get("isError")) {
                return Ok(1);
            }
        }
        Some("status") => {
            let s = call("/api/status", None).await?;
            let plugin = if truthy(s.get("pluginConnected")) {
                "connected".to_string()
            } else {
                "NOT connected — run the wridge plugin in Figma".to_string()
            };
            println!(
                "server: running (pid {})\nplugin: {plugin}\npaused: {}\ntools: {}",
                js_string(s.get("pid")),
                js_string(s.get("paused")),
                js_string(s.get("toolCount"))
            );
        }
        Some("list") => {
            let tools = call("/api/tools", None).await?;
            for t in tools.as_array().cloned().unwrap_or_default() {
                let params = t.get("params").cloned().unwrap_or(Value::Null);
                let props = params.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
                let required: Vec<String> = params
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|v| js_string(Some(v))).collect())
                    .unwrap_or_default();
                let list = props
                    .iter()
                    .map(|(k, v)| {
                        let opt = if required.contains(k) { "" } else { "?" };
                        let def = v.get("default").map(|d| format!("={}", stringify(d))).unwrap_or_default();
                        format!("{k}{opt}{def}")
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("{} {list}\n    {}", js_string(t.get("name")), js_string(t.get("description")));
            }
        }
        Some(tool) => {
            let args = match parse_args(&rest) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return Ok(1);
                }
            };
            let result = call("/api/tool", Some(&json!({"name": tool, "args": args}))).await?;
            for item in result.get("content").and_then(Value::as_array).cloned().unwrap_or_default() {
                if item.get("type") == Some(&json!("image")) {
                    let file = std::env::temp_dir().join(format!("wara-{tool}-{}.png", crate::guards::now_ms() as u64));
                    let data = item.get("data").and_then(Value::as_str).unwrap_or("");
                    let _ = std::fs::write(&file, crate::util::base64_lenient(data));
                    println!("image saved: {}", file.display());
                } else {
                    println!("{}", js_string(item.get("text")));
                }
            }
            if truthy(result.get("isError")) {
                return Ok(1);
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_args_parse_as_json_when_possible() {
        let args = parse_args(&["depth=2".into(), "nodeId=12:34".into(), "toNodeIds=[\"3:4\"]".into(), "flag=true".into()]).unwrap();
        assert_eq!(args, json!({"depth": 2, "nodeId": "12:34", "toNodeIds": ["3:4"], "flag": true}));
        assert_eq!(parse_args(&["{\"a\":".into(), "1}".into()]).unwrap(), json!({"a": 1}));
        assert!(parse_args(&["=x".into()]).is_err());
    }
}
