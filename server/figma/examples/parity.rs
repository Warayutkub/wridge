//! Rust side of the parity check (see parity/parity.mjs): runs every case through the crate and writes
//! `[{ id, ok: "<JSON text>" } | { id, error }]`, the same shape parity/node-run.mjs writes.
use anyhow::{anyhow, Result};
use serde_json::Value;
use wridge_figma::js::{self, stringify};
use wridge_figma::*;

fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&Value::Null)
}
fn opt(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i)
}
fn s(v: &Value) -> String {
    js::string(v)
}

fn run(module: &str, f: &str, args: &[Value]) -> Result<Value> {
    Ok(match (module, f) {
        ("lottie", "parsePath") => Value::Array(lottie::parse_path(&s(arg(args, 0))).iter().map(|p| p.to_json()).collect()),
        ("lottie", "parseColor") => lottie::Color::to_json(&lottie::parse_color(js::nullish(opt(args, 0)).map(s).as_deref())),
        ("lottie", "svgLayers") => {
            let split: Vec<String> = js::get(opt(args, 1), "split").and_then(|v| v.as_array()).map(|a| a.iter().map(s).collect()).unwrap_or_default();
            lottie::svg_layers(&s(arg(args, 0)), &split)?.to_json()
        }
        ("lottie", "compileLottie") => lottie::compile_lottie(arg(args, 0))?,

        ("motion-figma", "tracksToItems") => Value::Array(motion_figma::tracks_to_items(list(arg(args, 0)))?),
        ("motion-figma", "toEasing") => motion_figma::to_easing(opt(args, 0))?.unwrap_or(Value::Null),
        ("motion-figma", "presetsToItems") => Value::Array(motion_figma::presets_to_items(list(arg(args, 0)), list(arg(args, 1)))?),
        ("motion-figma", "motionText") => Value::String(motion_figma::motion_text(arg(args, 0))?),
        ("motion-figma", "motionToSpec") => motion_figma::motion_to_spec(arg(args, 0))?,

        ("motion", "listLottie") => Value::Array(motion::list_lottie(&s(arg(args, 0)))?),
        ("motion", "readLottie") => motion::read_lottie(&s(arg(args, 0)), &s(arg(args, 1)))?,
        ("motion", "insideProject") => Value::String(motion::inside_project(&s(arg(args, 0)), &s(arg(args, 1)))?),
        ("motion", "isLottie") => {
            let j = arg(args, 0);
            if js::truthy(Some(j)) { Value::Bool(motion::is_lottie(j)) } else { j.clone() }
        }
        ("motion", "contentHash") => Value::String(motion::content_hash(arg(args, 0))),
        ("motion", "markSentToFigma") => {
            let (dir, p) = (s(arg(args, 0)), s(arg(args, 1)));
            let mut out = motion::mark_sent_to_figma(&dir, &p, arg(args, 2))?;
            let mut written: Value = serde_json::from_str(&std::fs::read_to_string(node_path::join(&[&dir, &p]))?)?;
            // sentAt is the clock: blank it in both runs
            out["figma"]["sentAt"] = "<now>".into();
            written["meta"]["wara"]["figma"]["sentAt"] = "<now>".into();
            serde_json::json!({ "out": out, "written": written })
        }

        ("codegen", "resolveEntry") => codegen::resolve_entry(arg(args, 0), opt(args, 1)),
        ("codegen", "kebab") => Value::String(codegen::kebab(&s(arg(args, 0)))),
        ("codegen", "requireAbsolute") => Value::String(codegen::require_absolute(&s(arg(args, 0)), &s(arg(args, 1)))?),
        ("codegen", "formatTokens") => codegen::format_tokens(arg(args, 0), &s(arg(args, 1))),
        ("codegen", "annotateComponents") => {
            let mut root = arg(args, 0).clone();
            let unmapped = codegen::annotate_components(&mut root, arg(args, 1));
            serde_json::json!({ "unmapped": unmapped, "root": root })
        }
        ("codegen", "mergeBreakpoints") => codegen::merge_breakpoints(list(arg(args, 0)))?,
        ("codegen", "readComponentMap") => codegen::read_component_map(&s(arg(args, 0)))?,
        ("codegen", "writeComponentMap") => {
            let dir = s(arg(args, 0));
            let file = codegen::write_component_map(&dir, arg(args, 1))?;
            serde_json::json!({ "file": file.replacen(&dir, "<write>", 1), "text": std::fs::read_to_string(&file)? })
        }

        ("pdf", "pageNumber") => pdf::page_number(opt(args, 0)).map_or(Value::Null, js::num),
        ("pdf", "orderPages") => pdf::order_pages(opt(args, 0)),
        ("pdf", "pdfFileName") => Value::String(pdf::pdf_file_name(opt(args, 0))),
        ("pdf", "mergePdfs") => {
            let bufs = list(arg(args, 0)).iter().map(|f| std::fs::read(s(f))).collect::<std::io::Result<Vec<_>>>()?;
            let title = js::nullish(opt(args, 1)).map(s);
            serde_json::json!({ "base64": b64(&pdf::merge_pdfs(&bufs, title.as_deref())?) })
        }
        ("pdf", "designPdf") => {
            let nodes = arg(args, 0).clone();
            let asked = std::cell::RefCell::new(vec![]);
            let send = |cmd: String, a: Value, timeout: Option<u64>| {
                asked.borrow_mut().push(serde_json::json!([cmd, a, timeout]));
                let r: Result<Value> = if cmd == "get_node" {
                    Ok(nodes.get(a["nodeId"].as_str().unwrap_or("")).cloned().unwrap_or(Value::Null))
                } else {
                    let id = a["items"][0]["nodeId"].as_str().unwrap_or("").to_string();
                    let file = nodes["__pdf"].get(&id).or(nodes["__pdf"].get("default")).and_then(|f| f.as_str()).unwrap_or("").to_string();
                    std::fs::read(&file).map(|b| serde_json::json!([{ "base64": b64(&b) }])).map_err(Into::into)
                };
                std::future::ready(r)
            };
            let out = block_on(pdf::design_pdf(send, arg(args, 1)))?;
            serde_json::json!({ "title": out.title, "pages": out.pages, "warnings": out.warnings, "asked": asked.into_inner(), "base64": b64(&out.pdf) })
        }

        ("path", "resolve") => Value::String(node_path::resolve(&args.iter().map(s).collect::<Vec<_>>().iter().map(|x| x.as_str()).collect::<Vec<_>>())),
        ("path", "join") => Value::String(node_path::join(&args.iter().map(s).collect::<Vec<_>>().iter().map(|x| x.as_str()).collect::<Vec<_>>())),
        ("path", "normalize") => Value::String(node_path::normalize(&s(arg(args, 0)))),
        ("path", "isAbsolute") => Value::Bool(node_path::is_absolute(&s(arg(args, 0)))),
        ("path", "relative") => Value::String(node_path::relative(&s(arg(args, 0)), &s(arg(args, 1)))),

        ("figjam", "layoutDiagram") => figjam::layout_diagram(arg(args, 0))?,
        ("figjam", "boardText") => Value::String(figjam::board_text(arg(args, 0))?),

        ("motion-handoff", "motionContext") => Value::String(motion_handoff::motion_context(opt(args, 0)).into()),
        ("motion-handoff", "pipeline") => {
            use motion_handoff as h;
            let read = arg(args, 0);
            let context = s(arg(args, 1));
            let mut warnings = vec![];
            let rows = h::classify(read, &mut warnings, &context)?;
            let lints = h::lint(&rows, read, &context);
            let tk = h::tokens(&rows);
            let mut css_warnings = vec![];
            let css = h::to_css(&rows, &tk, &mut css_warnings);
            serde_json::json!({
                "rows": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
                "warnings": warnings, "lints": lints, "tokens": tk.to_json(), "css": css, "cssWarnings": css_warnings,
                "framer": h::to_framer(&rows, &tk), "check": h::check_text(read, &rows, &lints, &context),
            })
        }
        _ => return Err(anyhow!("no Rust runner for {module}.{f}")),
    })
}

/// `--numbers <file>`: lines "<f64 bits> <String(x) from Node>"; prints how many the crate writes differently.
fn numbers(file: &str) -> Result<()> {
    let text = std::fs::read_to_string(file)?;
    let (mut n, mut bad) = (0, 0);
    for line in text.lines() {
        let (bits, node) = line.split_once(' ').ok_or_else(|| anyhow!("bad line"))?;
        let x = f64::from_bits(bits.parse()?);
        n += 1;
        let rust = js::num_str(x);
        let back = js::str_to_number(node);
        if rust != node || back.to_bits() != x.to_bits() {
            bad += 1;
            if bad <= 5 {
                println!("  {x:?}: node {node} rust {rust}");
            }
        }
    }
    println!("numbers: {}/{n} written as JS writes them", n - bad);
    if bad > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "--numbers" {
        return numbers(&a[2]);
    }
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&a[1])?)?;
    // "<copy>" / "<write>": this runtime's own copy of the temp folders (see parity/cases-other.mjs)
    let out_dir = node_path::resolve(&[&a[1], ".."]);
    let place = |v: Value| match v.as_str() {
        Some("<copy>") => Value::String(node_path::join(&[&out_dir, "motion-project-rust"])),
        Some("<write>") => Value::String(node_path::join(&[&out_dir, "codegen-write-rust"])),
        _ => v,
    };
    let mut out = vec![];
    for c in &cases {
        let id = c["id"].clone();
        let args: Vec<Value> = c["args"].as_array().cloned().unwrap_or_default().into_iter().map(place).collect();
        let r = run(c["module"].as_str().unwrap_or(""), c["fn"].as_str().unwrap_or(""), &args);
        out.push(match r {
            Ok(v) => serde_json::json!({ "id": id, "ok": stringify(&v) }),
            Err(e) => serde_json::json!({ "id": id, "error": e.to_string() }),
        });
    }
    std::fs::write(&a[2], serde_json::to_string_pretty(&out)?)?;
    println!("rust: {} cases", out.len());
    Ok(())
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Runs a future that never waits (the parity `send` answers at once).
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}
