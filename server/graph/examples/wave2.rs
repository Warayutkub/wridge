//! `wave2 <calls.json>`: the Rust side of `parity/wave2.mjs`. Runs a list of calls
//! `[{ "fn": "<module>.<jsName>", "args": [...] }]` against the wave-2 modules (brain, memory,
//! notes, knowledge, flows, design, query, export) and prints one JSON line per call:
//! `{ "ok": <result> }` or `{ "error": "<message>" }`. Objects the JS returns by reference (graph,
//! componentLinks, tokenTable) are printed as the same projections `parity/node-wave2.mjs` prints.
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use wridge_graph::js::json_stringify;
use wridge_graph::query::Graph;
use wridge_graph::{brain, design, export, flows, knowledge, memory, notes, query};

fn st(v: Option<&Value>) -> String {
    wridge_graph::data::js_string(v)
}
fn opt_s(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        x => Some(st(x)),
    }
}
fn arg(a: &[Value], i: usize) -> Option<&Value> {
    a.get(i)
}
fn field<'a>(a: &'a [Value], i: usize, k: &str) -> Option<&'a Value> {
    a.get(i).and_then(|o| o.get(k))
}
fn strs(v: Option<&Value>) -> Option<Vec<String>> {
    v.and_then(Value::as_array).map(|a| a.iter().map(|x| st(Some(x))).collect())
}

fn with_graph<T>(dir: &str, f: impl FnOnce(&Graph) -> Result<T>) -> Result<T> {
    let (db, _) = wridge_graph::open_graph(dir)?;
    let g = query::graph(&db)?;
    f(&g)
}

fn sym_id(g: &Graph, si: Option<usize>) -> Value {
    si.map(|i| json!(g.syms[i].id)).unwrap_or(Value::Null)
}

fn graph_projection(g: &Graph) -> Value {
    let symbols: Vec<Value> = g.syms.iter().map(|s| json!([s.id, s.kind])).collect();
    let mut bindings = Vec::new();
    for r in &g.refs {
        let b = g.binding_target(r.file_id, &r.local);
        bindings.push(match b {
            None => Value::Null,
            Some(b) => json!({
                "external": b.external, "imported": b.imported, "unresolved": b.unresolved,
                "file": b.file.map(|f| g.files[f].path.clone()), "namespace": b.namespace, "symbol": sym_id(g, b.symbol),
            }),
        });
    }
    let refs_to: Vec<Value> = (0..g.syms.len())
        .map(|i| Value::Array(g.references_to(i).iter().map(|h| json!([g.refs[h.r].id, sym_id(g, h.owner)])).collect()))
        .collect();
    json!({ "files": g.files.len(), "symbols": symbols, "routes": g.routes.len(), "bindings": bindings, "referencesTo": refs_to })
}

fn links_projection(g: &Graph, l: &design::ComponentLinks) -> Value {
    json!({
        "linked": l.linked.iter().map(|x| json!({
            "figmaName": x.figma_name, "figma": x.figma, "entry": x.entry, "symbol": sym_id(g, x.symbol),
            "classes": x.classes.as_ref().map(|c| json!({ "defined": c.defined, "missing": c.missing, "files": c.files })),
        })).collect::<Vec<_>>(),
        "suggested": l.suggested.iter().map(|x| json!({ "figma": x.figma, "symbol": g.syms[x.symbol].id, "exact": x.exact })).collect::<Vec<_>>(),
        "figmaOnly": l.figma_only,
        "codeOnly": l.code_only.iter().map(|&i| g.syms[i].id).collect::<Vec<_>>(),
    })
}

fn table_projection(t: &design::TokenTable) -> Value {
    json!({
        "rows": t.rows.iter().map(|r| json!({
            "name": r.name,
            "defs": r.defs.iter().map(|d| json!({ "name": d.name, "value": d.value, "line": d.line, "path": d.path })).collect::<Vec<_>>(),
            "figma": r.figma,
            "uses": r.uses.iter().map(|u| json!({ "form": u.form, "name": u.name, "line": u.line, "count": u.count, "path": u.path })).collect::<Vec<_>>(),
            "files": r.files.iter().collect::<Vec<_>>(), "count": r.count, "resolved": r.resolved, "mismatch": r.mismatch,
        })).collect::<Vec<_>>(),
        "hardcoded": t.hardcoded.iter().map(|h| json!({ "name": h.name, "n": h.n, "files": h.files })).collect::<Vec<_>>(),
        "hardFiles": t.hard_files.iter().map(|h| json!({ "path": h.path, "n": h.n })).collect::<Vec<_>>(),
    })
}

fn root_of(dir: &str) -> Result<String> {
    Ok(wridge_graph::project_root(dir)?.to_string_lossy().into_owned())
}

fn call(name: &str, a: &[Value]) -> Result<Value> {
    let s0 = || st(arg(a, 0));
    let s1 = || st(arg(a, 1));
    Ok(match name {
        // ---------- brain ----------
        "brain.parseFrontMatter" => {
            let (data, body, offset) = brain::parse_front_matter(&s0());
            json!({ "data": Value::Object(data), "body": body, "offset": offset })
        }
        "brain.linkTarget" => {
            let (kind, target) = brain::link_target(&s0());
            json!({ "kind": kind, "target": target })
        }
        "brain.parseNote" => brain::parse_note(&s0(), &s1()).to_json(),
        "brain.brainSummary" => brain::brain_summary(&s0())?.unwrap_or(Value::Null),
        "brain.findNotes" => {
            let limit = field(a, 2, "limit").and_then(Value::as_u64).unwrap_or(8) as usize;
            Value::Array(brain::find_notes(&s0(), &opt_s(arg(a, 1)).unwrap_or_default(), limit)?)
        }
        "brain.readNote" => brain::read_note(&s0(), &s1())?,
        "brain.brainGraph" => brain::brain_graph(&s0())?,
        "brain.writeNote" => {
            let o = brain::WriteOptions {
                text: opt_s(field(a, 2, "text")),
                mode: opt_s(field(a, 2, "mode")).unwrap_or_else(|| "create".into()),
                tags: strs(field(a, 2, "tags")),
                keep_old: field(a, 2, "keepOld").and_then(Value::as_bool).unwrap_or(true),
            };
            brain::write_note(&s0(), &s1(), &o)?
        }
        "brain.trashNote" => brain::trash_note(&s0(), &s1())?,
        "brain.listTrash" => brain::list_trash(&s0())?,
        "brain.restoreNote" => brain::restore_note(&s0(), &s1())?,
        "brain.emptyTrash" => brain::empty_trash(&s0(), opt_s(arg(a, 1)).as_deref())?,
        "brain.codeLinkMissing" => json!(brain::code_link_missing(&s0(), &s1())?),
        "brain.notesAbout" => Value::Array(brain::notes_about(&s0(), opt_s(field(a, 1, "path")).as_deref(), opt_s(field(a, 1, "name")).as_deref())?),
        "brain.codeLinks" => Value::Array(brain::code_links(&s0())?),
        // ---------- memory ----------
        "memory.parseDecisions" => Value::Array(memory::parse_decisions(&s0()).iter().map(|d| d.to_json()).collect()),
        "memory.overview" => json!(memory::overview(&s0(), field(a, 1, "brief").and_then(Value::as_bool).unwrap_or(false))?),
        "memory.find" => {
            let limit = field(a, 2, "limit").and_then(Value::as_u64).unwrap_or(8) as usize;
            json!(memory::find(
                &s0(),
                &opt_s(arg(a, 1)).unwrap_or_default(),
                field(a, 2, "includeOld").and_then(Value::as_bool).unwrap_or(false),
                limit
            )?)
        }
        "memory.decide" => memory::decide(
            &s0(),
            &memory::DecideArgs {
                title: st(field(a, 1, "title")),
                decision: st(field(a, 1, "decision")),
                why: opt_s(field(a, 1, "why")),
                tags: strs(field(a, 1, "tags")),
                supersedes: opt_s(field(a, 1, "supersedes")),
            },
        )?,
        "memory.note" => memory::note(&s0(), opt_s(field(a, 1, "task")).as_deref(), &st(field(a, 1, "text")), strs(field(a, 1, "tags")).as_deref())?,
        "memory.handoff" => json!(memory::handoff(
            &s0(),
            &memory::HandoffArgs {
                task: st(field(a, 1, "task")),
                stage: opt_s(field(a, 1, "stage")),
                done: opt_s(field(a, 1, "done")),
                next: opt_s(field(a, 1, "next")),
                questions: opt_s(field(a, 1, "questions")),
                close: field(a, 1, "close").and_then(Value::as_bool).unwrap_or(false),
            }
        )?),
        "memory.findProjectRoot" => memory::find_project_root(&s0()).map(|x| json!(x)).unwrap_or(Value::Null),
        // ---------- knowledge ----------
        "knowledge.major" => json!(knowledge::major(&opt_s(arg(a, 0)).unwrap_or_default())),
        "knowledge.detectStack" => Value::Array(knowledge::detect_stack(&root_of(&s0())?)),
        "knowledge.listKnowledge" => knowledge::list_knowledge(&s0(), opt_s(arg(a, 1)).as_deref())?,
        "knowledge.getKnowledge" => knowledge::get_knowledge(&s0(), &s1(), &st(arg(a, 2)))?,
        "knowledge.readFacts" => knowledge::read_facts(&s0())?,
        "knowledge.setFact" => knowledge::set_fact(&s0(), &s1(), &st(arg(a, 2)))?,
        "knowledge.saveKnowledge" => knowledge::save_knowledge(
            &s0(),
            &knowledge::SaveArgs {
                topic: st(field(a, 1, "topic")),
                tool: st(field(a, 1, "tool")),
                version: opt_s(field(a, 1, "version")),
                content: st(field(a, 1, "content")),
                sources: strs(field(a, 1, "sources")),
            },
        )?,
        // ---------- flows ----------
        "flows.hashText" => json!(flows::hash_text(&s0())),
        "flows.parseFlow" => flows::parse_flow(&s0()),
        "flows.formatFlow" => json!(flows::format_flow(arg(a, 0).unwrap_or(&Value::Null))?),
        "flows.slugify" => json!(flows::slugify(&s0())),
        "flows.readFlowFiles" => Value::Array(flows::read_flow_files(&s0())?),
        "flows.saveFlowFile" => match flows::save_flow_file(&s0(), &s1(), &st(arg(a, 2)), arg(a, 3).unwrap_or(&Value::Null)) {
            Ok(v) => v,
            Err(e) => match e.downcast_ref::<flows::FlowConflict>() {
                Some(c) => json!({ "conflict": e.to_string(), "current": c.current }),
                None => return Err(e),
            },
        },
        "flows.readIgnored" => json!(flows::read_ignored(&s0()).into_iter().collect::<Vec<_>>()),
        "flows.ignoreDraft" => {
            flows::ignore_draft(&s0(), &s1())?;
            Value::Null
        }
        "flows.checkRef" => with_graph(&s0(), |g| Ok(flows::check_ref(g, &s1())))?,
        "flows.checkFlow" => with_graph(&s0(), |g| Ok(flows::check_flow(g, arg(a, 1).unwrap_or(&Value::Null))))?,
        "flows.trace" => with_graph(&s0(), |g| {
            let depth = field(a, 2, "depth").and_then(Value::as_i64).unwrap_or(3);
            let max = field(a, 2, "max").and_then(Value::as_u64).unwrap_or(40) as usize;
            flows::trace(g, &s1(), depth, max)
        })?,
        "flows.statusValues" => with_graph(&s0(), |g| {
            let tables = strs(arg(a, 2)).unwrap_or_default().into_iter().collect();
            Ok(Value::Array(flows::status_values(g, &s1(), &tables)?))
        })?,
        "flows.draftFlows" => with_graph(&s0(), |g| Ok(Value::Array(flows::draft_flows(g, &s1(), wridge_graph_arr(arg(a, 2)))?)))?,
        "flows.flowsOverview" => with_graph(&s0(), |g| flows::flows_overview(g, &root_of(&s0())?))?,
        "flows.flowsText" => with_graph(&s0(), |g| {
            let limit = arg(a, 3).and_then(Value::as_i64).unwrap_or(40);
            Ok(json!(flows::flows_text(g, &s1(), opt_s(arg(a, 2)).as_deref(), limit)?))
        })?,
        "flows.traceText" => with_graph(&s0(), |g| Ok(json!(flows::trace_text(g, &s1())?)))?,
        // ---------- query ----------
        "query.runQuery" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            let o = query::QueryOptions {
                depth: field(a, 3, "depth").and_then(Value::as_i64),
                limit: field(a, 3, "limit").and_then(Value::as_i64).unwrap_or(40),
                root: opt_s(field(a, 3, "root")),
            };
            json!(query::run_query(&db, &s1(), opt_s(arg(a, 2)).as_deref(), &o)?)
        }
        "query.graph" => with_graph(&s0(), |g| Ok(graph_projection(g)))?,
        // ---------- notes ----------
        "notes.resolveNoteTarget" => with_graph(&s0(), |g| Ok(notes::resolve_note_target(g, &s1())?.to_json()))?,
        "notes.writeNote" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            let g = query::graph(&db)?;
            notes::write_note(&db, &g, &s1(), opt_s(arg(a, 2)).as_deref())?
        }
        "notes.allNotes" => with_graph(&s0(), |g| Ok(Value::Array(notes::all_notes(g.db, g)?.iter().map(|n| n.to_json()).collect())))?,
        "notes.missingNotes" => with_graph(&s0(), |g| notes::missing_notes(g.db, g, arg(a, 1).and_then(Value::as_i64).unwrap_or(15)))?,
        "notes.notesFor" => with_graph(&s0(), |g| {
            let f = g.file_by_path_exact(&s1()).ok_or_else(|| anyhow!("no file"))?;
            Ok(Value::Array(notes::notes_for(g.db, g, &g.files[f])?.iter().map(|n| n.to_json()).collect()))
        })?,
        // ---------- design ----------
        "design.designSnapshot" => with_graph(&s0(), |g| Ok(design::design_snapshot(g.db).unwrap_or(Value::Null)))?,
        "design.componentLinks" => with_graph(&s0(), |g| {
            let snap = design::design_snapshot(g.db);
            Ok(links_projection(g, &design::component_links(g, &s1(), snap.as_ref())?))
        })?,
        "design.tokenTable" => with_graph(&s0(), |g| {
            let snap = design::design_snapshot(g.db);
            Ok(table_projection(&design::token_table(g.db, g, snap.as_ref())?))
        })?,
        "design.importPathFor" => with_graph(&s0(), |g| {
            let f = g.file_by_path_exact(&st(arg(a, 2))).ok_or_else(|| anyhow!("no file"))?;
            Ok(json!(design::import_path_for(g, &s1(), &g.files[f])))
        })?,
        "design.confirmLink" => with_graph(&s0(), |g| {
            design::confirm_link(g, &s1(), &st(arg(a, 2)), arg(a, 3).and_then(Value::as_f64).unwrap_or(f64::NAN))
        })?,
        "design.syncDesign" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            let answers = [
                arg(a, 1).cloned().unwrap_or(Value::Null),
                arg(a, 2).cloned().unwrap_or(Value::Null),
                arg(a, 3).cloned().unwrap_or(Value::Null),
            ];
            let mut sent: Vec<Value> = Vec::new();
            let mut i = 0;
            let r = design::sync_design(&db, |cmd, params, timeout| {
                sent.push(json!([cmd, params, timeout]));
                i += 1;
                Ok(answers[i - 1].clone())
            })?;
            json!({ "result": r, "sent": sent, "snapshot": design::design_snapshot(&db) })
        }
        // ---------- export ----------
        "export.exportData" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            export::export_data(&db, &s1())?
        }
        "export.exportGraph" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            export::export_graph(&db, &s1())?
        }
        // ---------- HTTP read paths (index.mjs) ----------
        "http.brain" => brain::brain_graph(&root_of(&s0())?)?,
        "http.codeLinks" => Value::Array(brain::code_links(&root_of(&s0())?)?),
        "http.note" => brain::read_note(&root_of(&s0())?, &s1())?,
        "http.trash" => brain::list_trash(&root_of(&s0())?)?,
        "http.flows" => {
            let (db, _) = wridge_graph::open_graph(&s0())?;
            let g = query::graph(&db)?;
            let root = root_of(&s0())?;
            let mut o = Map::new();
            o.insert("name".into(), json!(root.replace('\\', "/").split('/').next_back().unwrap_or("")));
            if let Value::Object(ov) = flows::flows_overview(&g, &root)? {
                o.extend(ov);
            }
            Value::Object(o)
        }
        "http.flowsTrace" => with_graph(&s0(), |g| flows::trace(g, &s1(), 3, 40))?,
        _ => return Err(anyhow!("unknown call {name}")),
    })
}

fn wridge_graph_arr(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

fn main() {
    let file = std::env::args().nth(1).expect("usage: wave2 <calls.json>");
    let calls: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    for c in calls {
        let name = c.get("fn").and_then(Value::as_str).unwrap_or("");
        let args: Vec<Value> = c.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        let t0 = std::time::Instant::now();
        let out = match call(name, &args) {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "error": e.to_string() }),
        };
        if std::env::var("WAVE2_TIMES").is_ok() {
            eprintln!("{:>6} ms {name}", t0.elapsed().as_millis());
        }
        println!("{}", json_stringify(&out));
    }
}
