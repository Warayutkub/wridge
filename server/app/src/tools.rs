//! Port of tools.mjs: the tool registry (names, descriptions, input schemas) and the handlers.
//! Plain pass-through tools share the plugin command name. The design-side logic (codegen, lottie, motion,
//! figjam, pdf) is wridge-figma's; the code graph, project memory, brain and knowledge are wridge-graph's.

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use futures::FutureExt;
use serde_json::{json, Map, Value};
use wridge_figma::codegen::{self, BREAKPOINTS, COMPONENT_MAP_FILE};
use wridge_figma::figjam::{self, DIAGRAM_TYPES, NODE_KINDS, NODE_STATUS};
use wridge_figma::motion_figma::{self, MOTION_PROPS};
use wridge_figma::motion_handoff::{self, Row};
use wridge_figma::{js as fjs, lottie, node_path, obj, pdf};

use crate::app::Ctx;
use crate::jsjson::{first_of, js_string, len_utf16, length, prop, stringify, truthy};
use crate::codegraph::{self, blocking};
use wridge_graph as wg;
use wridge_graph::query::QUERIES;
use crate::zod::*;

pub type Handler = Arc<dyn Fn(Ctx, Value) -> BoxFuture<'static, anyhow::Result<Value>> + Send + Sync>;

pub struct Tool {
    pub name: String,
    pub description: String,
    pub shape: Shape,
    pub run: Handler,
}

pub fn handler<F, Fut>(f: F) -> Handler
where
    F: Fn(Ctx, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    Arc::new(move |c, a| f(c, a).boxed())
}

// ---------- results ----------

/// `{ content: [{ type: "text", text }] }`
pub fn text_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

/// `text(value)`: compact JSON (indentation costs tokens on every result).
pub fn text(value: &Value) -> Value {
    text_result(stringify(value))
}

/// `text(value)` for values built from wridge-figma results: exactly the bytes `JSON.stringify` writes
/// (integer-like keys first, as JS objects enumerate them).
pub fn js_text(value: &Value) -> Value {
    text_result(fjs::stringify(value))
}

/// `plain(value)`: strings as they are, anything else as JSON.
pub fn plain(value: &Value) -> Value {
    match value {
        Value::String(s) => text_result(s.clone()),
        v => text_result(stringify(v)),
    }
}

/// `text(value)` for wridge-graph results (`wridge_graph::js::json_stringify`).
pub fn graph_text(value: &Value) -> Value {
    text_result(codegraph::stringify(value))
}

/// `plain(value)` for wridge-graph results.
pub fn graph_plain(value: &Value) -> Value {
    match value {
        Value::String(s) => text_result(s.clone()),
        v => graph_text(v),
    }
}

/// An optional string argument (zod already checked the type).
fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// An optional string-array argument.
fn opt_strings(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(Value::as_array).map(|a| a.iter().map(|v| js_string(Some(v))).collect())
}

/// project_memory read: one whole note, then its links and backlinks (empty lines dropped, as
/// `.filter(Boolean)` does).
fn note_text(r: &Value) -> String {
    let links: Vec<String> = r["links"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|l| {
            let rel = if truthy(l.get("rel")) { format!("{}: ", js_string(l.get("rel"))) } else { String::new() };
            let kind = if l.get("kind") == Some(&json!("note")) { String::new() } else { format!("{}:", js_string(l.get("kind"))) };
            let missing = if truthy(l.get("missing")) { " (missing)" } else { "" };
            format!("{rel}{kind}{}{missing}", js_string(l.get("target")))
        })
        .collect();
    let backlinks: Vec<String> = r["backlinks"].as_array().map(|a| a.iter().map(|b| js_string(Some(b))).collect()).unwrap_or_default();
    let mut lines = vec![
        format!("# {} — {}", js_string(r.get("note")), js_string(r.get("path"))),
        crate::jsjson::js_trim(&js_string(r.get("text"))).to_string(),
    ];
    if !links.is_empty() {
        lines.push(format!("Links: {}", links.join(" · ")));
    }
    if !backlinks.is_empty() {
        lines.push(format!("Linked from: {}", backlinks.join(" · ")));
    }
    lines.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn get<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.get(key)
}

fn get_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `const { a, b, ...rest } = args`
fn rest(args: &Value, without: &[&str]) -> Map<String, Value> {
    args.as_object()
        .map(|o| o.iter().filter(|(k, _)| !without.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

// ---------- shared schemas ----------

fn hex() -> Schema {
    string().regex_msg("^#([0-9a-f]{6}|[0-9a-f]{8})$", "i", "hex color like #FF8866 or #FF886680")
}
fn hex_or_none() -> Schema {
    union(vec![hex(), literal(json!("none"))])
}
fn node_id() -> Schema {
    string().describe("Figma node id, e.g. 12:34")
}
fn node_ids() -> Schema {
    array(string()).min(1.0)
}
fn allow_flow_issues() -> Schema {
    boolean()
        .optional()
        .describe("Only when the user accepts sending this section to review/approval with prototype issues")
}
fn parent_id() -> Schema {
    string().optional().describe("Parent frame id; defaults to the current page")
}
fn px() -> Schema {
    number().min(0.0)
}
fn sizing() -> Schema {
    enumeration(&["FIXED", "HUG", "FILL"])
}

fn layout_props() -> Shape {
    shape()
        .f("layoutMode", enumeration(&["NONE", "HORIZONTAL", "VERTICAL"]).optional())
        .f("itemSpacing", px().optional().describe("Gap between children"))
        .f("counterAxisSpacing", px().optional().describe("Gap between wrapped rows"))
        .f("wrap", boolean().optional().describe("Wrap children (horizontal only)"))
        .f("padding", px().optional().describe("All four sides"))
        .f("paddingX", px().optional())
        .f("paddingY", px().optional())
        .f("paddingTop", px().optional())
        .f("paddingRight", px().optional())
        .f("paddingBottom", px().optional())
        .f("paddingLeft", px().optional())
        .f(
            "primaryAlign",
            enumeration(&["MIN", "CENTER", "MAX", "SPACE_BETWEEN"]).optional().describe("Along the layout direction"),
        )
        .f(
            "counterAlign",
            enumeration(&["MIN", "CENTER", "MAX", "BASELINE"]).optional().describe("Across the layout direction"),
        )
        .build()
}

fn sizing_props() -> Shape {
    shape()
        .f(
            "widthSizing",
            sizing().optional().describe("HUG = fit content, FILL = stretch in auto-layout parent, FIXED"),
        )
        .f("heightSizing", sizing().optional())
        .f("minWidth", px().optional())
        .f("maxWidth", px().optional())
        .f("minHeight", px().optional())
        .f("maxHeight", px().optional())
        .build()
}

fn corner_props() -> Shape {
    shape()
        .f("cornerRadius", px().optional())
        .f("topLeft", px().optional())
        .f("topRight", px().optional())
        .f("bottomRight", px().optional())
        .f("bottomLeft", px().optional())
        .build()
}

fn line_height() -> Schema {
    union(vec![number().positive(), string().regex("^(auto|\\d+(\\.\\d+)?%)$", "i")]).describe("px number, \"150%\", or \"auto\"")
}
fn letter_spacing() -> Schema {
    union(vec![number(), string().regex("^-?\\d+(\\.\\d+)?%$", "")]).describe("px number or \"-2%\"")
}

fn text_props() -> Shape {
    shape()
        .f("fontSize", number().positive().optional())
        .f("lineHeight", line_height().optional())
        .f("letterSpacing", letter_spacing().optional())
        .f("textAlign", enumeration(&["LEFT", "CENTER", "RIGHT", "JUSTIFIED"]).optional())
        .f("autoResize", enumeration(&["NONE", "WIDTH_AND_HEIGHT", "HEIGHT", "TRUNCATE"]).optional())
        .f("width", number().positive().optional().describe("Fixed width; text wraps and grows in height"))
        .f("fill", hex().optional())
        .build()
}

const BINDABLE_FIELDS: &[&str] = &[
    "fill",
    "stroke",
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "itemSpacing",
    "counterAxisSpacing",
    "paddingLeft",
    "paddingRight",
    "paddingTop",
    "paddingBottom",
    "cornerRadius",
    "topLeftRadius",
    "topRightRadius",
    "bottomLeftRadius",
    "bottomRightRadius",
    "strokeWeight",
    "opacity",
    "visible",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontWeight",
    "letterSpacing",
    "lineHeight",
];

// ---------- images ----------

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

fn image_format(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, 0x50, 0x4e, 0x47, ..] => Some("png"),
        [0xff, 0xd8, 0xff, ..] => Some("jpeg"),
        [0x47, 0x49, 0x46, ..] => Some("gif"),
        _ => None,
    }
}

async fn load_image(source: &str) -> anyhow::Result<(String, &'static str, usize)> {
    let is_url = regress::Regex::with_flags("^https?:\\/\\/", "i").unwrap().find(source).is_some();
    let bytes = if is_url {
        let res = reqwest::get(source).await.map_err(|_| anyhow!("fetch failed"))?;
        if !res.status().is_success() {
            return Err(anyhow!("Download failed: HTTP {} for {source}", res.status().as_u16()));
        }
        res.bytes().await.map_err(|_| anyhow!("terminated"))?.to_vec()
    } else {
        crate::util::read_file(Path::new(source))?
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(anyhow!("Image is {:.1} MB; limit is 20 MB", bytes.len() as f64 / 1e6));
    }
    let Some(format) = image_format(&bytes) else {
        return Err(anyhow!("Figma only accepts PNG, JPEG or GIF images (WebP/SVG/HEIC are not supported)"));
    };
    Ok((crate::util::base64_encode(&bytes), format, bytes.len()))
}

// ---------- design_map as text ----------

fn list_with_more(items: &[Value], max: usize, fmt: impl Fn(&Value) -> String) -> String {
    let mut s = items.iter().take(max).map(fmt).collect::<Vec<_>>().join(", ");
    if items.len() > max {
        s.push_str(&format!(", +{} more", items.len() - max));
    }
    s
}

fn arr(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn t(v: Option<&Value>) -> String {
    js_string(v)
}

/// Plain-text overview: far fewer tokens than the equivalent JSON.
pub fn render_design_map(map: &Value, max_screens: usize) -> String {
    let pages = arr(map.get("pages"));
    let components = arr(map.get("components"));
    let screens: usize = pages.iter().map(|p| arr(p.get("screens")).len()).sum();
    let mut lines = vec![format!(
        "File \"{}\" · {} page(s) · {} screens · {} components in use · Foundations: {}",
        t(map.get("file")),
        pages.len(),
        screens,
        components.len(),
        match map.get("foundations") {
            Some(Value::String(w)) if !w.is_empty() => w.clone(),
            Some(Value::Bool(true)) => "yes".into(),
            _ => "none (get_tokens reads a page, section or frame named \"Foundations\")".into(),
        }
    )];
    let all: Vec<Value> = pages.iter().flat_map(|p| arr(p.get("screens"))).collect();
    let count = |st: &str| all.iter().filter(|s| s.get("status") == Some(&Value::String(st.into()))).count();
    lines.push(format!(
        "Prototype work only (sections \"<Flow> · Prototype · Draft / In review / Approved\"; old \"<Flow> · <Status>\" and loose frames count too): {} approved · {} in review · {} draft · {} without a review section (not approved)",
        count("approved"),
        count("review"),
        count("draft"),
        all.iter().filter(|s| !truthy(s.get("status"))).count()
    ));
    for page in &pages {
        let page_screens = arr(page.get("screens"));
        lines.push(String::new());
        lines.push(format!("## Page \"{}\" ({}) — {} screens", t(page.get("name")), t(page.get("id")), page_screens.len()));
        let flows = arr(page.get("flows"));
        if !flows.is_empty() {
            lines.push(format!(
                "Flows: {}",
                flows
                    .iter()
                    .map(|f| format!("{} → {}", t(f.get("name")), t(f.get("nodeId"))))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        for s in page_screens.iter().take(max_screens) {
            let status = if truthy(s.get("status")) {
                format!(" · {}", t(s.get("status")).to_uppercase())
            } else {
                String::new()
            };
            let mut line = format!("- {} ({}) {}{}", t(s.get("name")), t(s.get("id")), t(s.get("size")), status);
            let comps = arr(s.get("components"));
            if !comps.is_empty() {
                line.push_str(&format!(
                    " · uses {}",
                    list_with_more(&comps, 6, |c| {
                        let n = t(c.get(0));
                        let k = c.get(1).and_then(Value::as_f64).unwrap_or(f64::NAN);
                        if k > 1.0 {
                            format!("{n}×{}", t(c.get(1)))
                        } else {
                            n
                        }
                    })
                ));
            }
            let to = arr(s.get("to"));
            if !to.is_empty() {
                line.push_str(&format!(" · → {}", list_with_more(&to, 5, |x| t(Some(x)))));
            }
            lines.push(line);
        }
        if page_screens.len() > max_screens {
            lines.push(format!("… {} more screens (pass pageId or maxScreens)", page_screens.len() - max_screens));
        }
    }
    if let Some(Value::Object(sets)) = map.get("breakpoints") {
        if !sets.is_empty() {
            lines.push(String::new());
            lines.push("## Breakpoint sets".into());
            for (name, bps) in sets {
                let list = bps
                    .as_object()
                    .map(|o| o.iter().map(|(bp, id)| format!("{bp} {}", t(Some(id)))).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                lines.push(format!("- {name}: {list}"));
            }
        }
    }
    if !components.is_empty() {
        lines.push(String::new());
        lines.push("## Components (instances / screens)".into());
        lines.push(
            components
                .iter()
                .map(|c| format!("{} {}/{}", t(c.get(0)), t(c.get(1)), t(c.get(2))))
                .collect::<Vec<_>>()
                .join(" · "),
        );
    }
    let unused = arr(map.get("unusedComponents"));
    if !unused.is_empty() {
        lines.push(String::new());
        lines.push(format!("Unused local components: {}", crate::jsjson::js_join(&unused, ", ")));
    }
    let other = arr(map.get("otherWork"));
    if !other.is_empty() {
        lines.push(String::new());
        lines.push("## Other work (not in the design graph)".into());
        for s in other {
            lines.push(format!("- {} [{}]", t(s.get("name")), t(s.get("kind"))));
        }
    }
    lines.join("\n")
}

// ---------- registry ----------

struct Defs(Vec<Tool>);

impl Defs {
    /// `tool(name, description, inputSchema)`: forwards the arguments to the plugin command of the same name.
    fn fwd(&mut self, name: &str, description: &str, shape: Shape) {
        let command = name.to_string();
        let run = handler(move |ctx: Ctx, args: Value| {
            let command = command.clone();
            async move { Ok(text(&ctx.figma(&command, args, None).await?)) }
        });
        self.custom(name, description, shape, run);
    }
    fn custom(&mut self, name: &str, description: &str, shape: Shape, run: Handler) {
        self.0.push(Tool {
            name: name.into(),
            description: description.into(),
            shape,
            run,
        });
    }
    /// graphTool: a plugin call with the design-graph timeout and a renderer.
    fn graph(&mut self, name: &str, description: &str, shape: Shape, render: fn(Value, &Value) -> Value) {
        let command = name.to_string();
        let run = handler(move |ctx: Ctx, args: Value| {
            let command = command.clone();
            async move {
                let v = ctx.figma(&command, args.clone(), Some(GRAPH_TIMEOUT_MS)).await?;
                Ok(render(v, &args))
            }
        });
        self.custom(name, description, shape, run);
    }
}

const GRAPH_TIMEOUT_MS: u64 = 120_000;

fn project_dir_desc() -> String {
    format!("Absolute path of the code project (holds {COMPONENT_MAP_FILE})")
}

/// Returns the tools of tools.mjs in the same order.
pub fn define_tools() -> Vec<Tool> {
    let mut d = Defs(vec![]);
    let empty = || shape().build();

    // ---- read / discover ----

    d.fwd("get_document_info", "Current file, page list, and the top-level nodes of the current page.", empty());

    d.fwd(
        "get_selection",
        "Nodes the user currently has selected, with children down to `depth`.",
        shape().f("depth", int().min(0.0).max(5.0).default(json!(1))).build(),
    );

    d.fwd(
        "get_node",
        "Read one node: geometry, fills, images, text, auto layout, sizing, styles, component props, children to `depth`.",
        shape().f("nodeId", node_id()).f("depth", int().min(0.0).max(5.0).default(json!(2))).build(),
    );

    d.fwd(
        "find_nodes",
        "Search layers by name (case-insensitive substring) and/or type on the current page or inside `withinId`.",
        shape()
            .f("name", string().optional())
            .f("type", string().optional().describe("FRAME, TEXT, INSTANCE, COMPONENT, RECTANGLE, GROUP…"))
            .f("withinId", string().optional())
            .f("limit", int().min(1.0).max(500.0).default(json!(50)))
            .build(),
    );

    d.custom(
        "export_node_image",
        "Render a node to PNG to check how it looks. Use after building each section. Images cost many tokens: keep maxSize small unless you need detail.",
        shape()
            .f("nodeId", node_id())
            .f("scale", number().min(0.25).max(3.0).default(json!(1)))
            .f(
                "maxSize",
                int()
                    .min(128.0)
                    .max(4096.0)
                    .default(json!(1024))
                    .describe("Longest side in px; the render is scaled down to fit"),
            )
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let r = ctx.figma("export_node_image", args, None).await?;
            let mut item = Map::new();
            item.insert("type".into(), "image".into());
            if let Some(b) = r.get("base64") {
                item.insert("data".into(), b.clone());
            }
            item.insert("mimeType".into(), "image/png".into());
            Ok(json!({"content": [Value::Object(item)]}))
        }),
    );

    d.fwd(
        "get_styles",
        "Local paint, text and effect styles in this file. Check these before picking raw colors/fonts.",
        empty(),
    );

    d.fwd(
        "get_variables",
        "Local variable collections, modes and variables (colors as hex). Prefer these over raw values.",
        empty(),
    );

    d.fwd(
        "get_components",
        "Local components and component sets (with variant options and properties). Reuse these before drawing new UI.",
        shape().f("allPages", boolean().default(json!(false))).build(),
    );

    // ---- create ----

    d.fwd(
        "create_frame",
        "Create a frame. Auto layout is VERTICAL by default — pass layoutMode NONE only for free positioning.",
        shape()
            .f("name", string().default(json!("Frame")))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().default(json!(375)))
            .f("height", number().positive().default(json!(812)))
            .f("parentId", parent_id())
            .f("fill", hex().optional().describe("Omit for a transparent frame"))
            .f("clip", boolean().optional())
            .spread(&corner_props())
            .spread(&layout_props())
            .f("layoutMode", enumeration(&["NONE", "HORIZONTAL", "VERTICAL"]).default(json!("VERTICAL")))
            .spread(&sizing_props())
            .build(),
    );

    d.fwd(
        "create_rectangle",
        "Create a rectangle (dividers, image placeholders, decorative shapes).",
        shape()
            .f("name", string().default(json!("Rectangle")))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().default(json!(100)))
            .f("height", number().positive().default(json!(100)))
            .f("parentId", parent_id())
            .f("fill", hex().optional())
            .spread(&corner_props())
            .spread(&sizing_props())
            .build(),
    );

    d.fwd(
        "create_ellipse",
        "Create an ellipse/circle (avatars, dots, icons backgrounds).",
        shape()
            .f("name", string().default(json!("Ellipse")))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().default(json!(40)))
            .f("height", number().positive().default(json!(40)))
            .f("parentId", parent_id())
            .f("fill", hex().optional())
            .spread(&sizing_props())
            .build(),
    );

    d.fwd(
        "create_text",
        "Create a text layer. Use a Thai-capable font (IBM Plex Sans Thai, Noto Sans Thai, Prompt…) for Thai text.",
        shape()
            .f("text", string())
            .f("name", string().optional())
            .f("x", number().optional())
            .f("y", number().optional())
            .f("parentId", parent_id())
            .f("fontFamily", string().default(json!("Inter")))
            .f("fontStyle", string().default(json!("Regular")).describe("Regular, Medium, SemiBold, Bold…"))
            .spread(&text_props())
            .f("fontSize", number().positive().default(json!(16)))
            .spread(&sizing_props())
            .build(),
    );

    d.custom(
        "create_instance",
        "Place an instance of a component (by componentId from get_components, or a published componentKey).",
        shape()
            .f(
                "componentId",
                string().optional().describe("COMPONENT or COMPONENT_SET id (a set uses its default variant)"),
            )
            .f("componentKey", string().optional())
            .f("parentId", parent_id())
            .f("x", number().optional())
            .f("y", number().optional())
            .f("properties", record(union(vec![string(), boolean()])).optional().describe("Variant/text/boolean props"))
            .spread(&sizing_props())
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            if !truthy(get(&args, "componentId")) && !truthy(get(&args, "componentKey")) {
                return Err(anyhow!("Pass componentId or componentKey"));
            }
            Ok(text(&ctx.figma("create_instance", args, None).await?))
        }),
    );

    d.fwd(
        "create_component",
        "Turn an existing frame/node into a reusable component.",
        shape().f("nodeId", node_id()).f("name", string().optional()).f("description", string().optional()).build(),
    );

    d.fwd(
        "combine_as_variants",
        "Combine components into one component set. Name each component \"Prop=Value, Prop=Value\" first (e.g. \"Type=Primary, State=Hover\") — those become the variant properties. Lays the set out as a wrapping grid unless layout is false.",
        shape()
            .f("nodeIds", array(string()).min(2.0).describe("COMPONENT ids"))
            .f("name", string().min(1.0).describe("Set name = code component name, e.g. \"Button\""))
            .f("parentId", parent_id())
            .f("layout", boolean().default(json!(true)))
            .f("width", number().positive().default(json!(960)))
            .f("gap", number().min(0.0).default(json!(16)))
            .f("padding", number().min(0.0).default(json!(24)))
            .build(),
    );

    d.fwd(
        "create_section",
        "Wrap top-level frames of one piece of work in a Figma section (free plan; not a page). Name = \"<Name> · <Kind> · <Status>\": kind Prototype (app screens: in the design graph, prototype checked before review) or Slides / Storyboard / Graphic / anything else (not in the design graph, no prototype check); status Draft, In review or Approved. Change status later with rename_node on the section.",
        shape()
            .f("name", string().min(1.0).describe("e.g. \"Payroll · Draft\""))
            .f("nodeIds", array(string()).optional().describe("Top-level frames to put inside; the section is sized around them"))
            .f("padding", number().min(0.0).default(json!(80)))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().optional())
            .f("height", number().positive().optional())
            .f("allowFlowIssues", allow_flow_issues())
            .build(),
    );

    d.fwd(
        "add_component_property",
        "Add a TEXT (label), BOOLEAN (show/hide a layer) or INSTANCE_SWAP (icon) property to a component or component set, bound to layers inside it (bindNodeIds; for a set, bind the matching layer in every variant). Mirror the code component's props.",
        shape()
            .f("componentId", string().describe("COMPONENT or COMPONENT_SET id (a variant resolves to its set)"))
            .f("name", string().min(1.0).describe("Property name, e.g. \"Label\", \"Show icon\""))
            .f("type", enumeration(&["TEXT", "BOOLEAN", "INSTANCE_SWAP"]))
            .f("defaultValue", union(vec![string(), boolean()]).describe("TEXT: the text; BOOLEAN: true/false; INSTANCE_SWAP: a component id"))
            .f("bindNodeIds", array(string()).optional())
            .build(),
    );

    d.fwd(
        "create_paint_style",
        "Create a local color style (use slash names like Color/Primary/500).",
        shape().f("name", string().min(1.0)).f("color", hex()).build(),
    );

    d.fwd(
        "create_text_style",
        "Create a local text style (e.g. Text/Heading/H1).",
        shape()
            .f("name", string().min(1.0))
            .f("fontFamily", string())
            .f("fontStyle", string().default(json!("Regular")))
            .f("fontSize", number().positive())
            .f("lineHeight", line_height().optional())
            .f("letterSpacing", letter_spacing().optional())
            .build(),
    );

    d.fwd(
        "duplicate_node",
        "Duplicate a node (optionally into another parent / position).",
        shape()
            .f("nodeId", node_id())
            .f("parentId", parent_id())
            .f("x", number().optional())
            .f("y", number().optional())
            .f("name", string().optional())
            .build(),
    );

    d.fwd(
        "group_nodes",
        "Group nodes (prefer auto-layout frames over groups for UI).",
        shape().f("nodeIds", node_ids()).f("name", string().optional()).build(),
    );

    d.fwd("ungroup_node", "Ungroup a group.", shape().f("nodeId", node_id()).build());

    // ---- modify ----

    d.fwd(
        "set_text",
        "Replace the characters of a text node.",
        shape().f("nodeId", node_id()).f("text", string()).build(),
    );

    d.fwd(
        "set_text_props",
        "Change typography of a text node.",
        shape()
            .f("nodeId", node_id())
            .f("fontFamily", string().optional())
            .f("fontStyle", string().optional())
            .spread(&text_props())
            .build(),
    );

    d.fwd(
        "set_fill",
        "Set a single solid fill, or \"none\" to remove fills.",
        shape().f("nodeId", node_id()).f("color", hex_or_none()).build(),
    );

    d.fwd(
        "set_stroke",
        "Set a layer's stroke — everything in Figma's Stroke settings: color (\"none\" removes), weight, align; Basic: dash [dash, gap] or \"solid\", cap (both ends) or startCap / endCap (open vectors), join, miterAngle, widthProfile (taper, wedge…) or widthPoints; Dynamic: dynamic {frequency, wiggle, smoothen}; Brush: brush {name, direction, …}. kind: \"basic\" turns dynamic/brush off.",
        shape()
            .f("nodeId", node_id())
            .f("color", hex_or_none().optional())
            .f("weight", px().optional())
            .f("align", enumeration(&["INSIDE", "OUTSIDE", "CENTER"]).optional())
            .f("dash", union(vec![array(number().min(0.0)), literal(json!("solid"))]).optional().describe("[dash, gap] in px, e.g. [8, 4]; \"solid\" removes dashes"))
            .f("cap", enumeration(&["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL", "DIAMOND_FILLED", "TRIANGLE_FILLED", "CIRCLE_FILLED"]).optional().describe("Both ends"))
            .f("startCap", enumeration(&["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL", "DIAMOND_FILLED", "TRIANGLE_FILLED", "CIRCLE_FILLED"]).optional().describe("Start of an open vector path"))
            .f("endCap", enumeration(&["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL", "DIAMOND_FILLED", "TRIANGLE_FILLED", "CIRCLE_FILLED"]).optional().describe("End of an open vector path"))
            .f("join", enumeration(&["MITER", "BEVEL", "ROUND"]).optional())
            .f("miterAngle", number().min(1.0).max(180.0).optional().describe("Degrees; sharper corners than this are beveled"))
            .f("widthProfile", enumeration(&["UNIFORM", "WEDGE", "TAPER", "QUARTER_TAPER", "EYE", "MIRRORED_TAPER"]).optional().describe("Variable width preset (not with dynamic)"))
            .f("widthPoints", array(object(shape().f("position", number().min(0.0).max(1.0)).f("width", number().min(0.0)).build())).optional().describe("Custom variable width: position 0–1 along the stroke, width as a fraction of weight (1 = weight)"))
            .f("kind", enumeration(&["basic", "dynamic", "brush"]).optional().describe("Stroke settings tab; basic clears dynamic / brush"))
            .f(
                "dynamic",
                object(
                    shape()
                        .f("frequency", number().min(0.01).max(20.0).optional())
                        .f("wiggle", number().min(0.0).optional())
                        .f("smoothen", number().min(0.0).max(1.0).optional())
                        .build(),
                )
                .optional()
                .describe("Hand-drawn wobble (Stroke settings → Dynamic)"),
            )
            .f(
                "brush",
                object(
                    shape()
                        .f("name", string().describe("Stretch: HEIST, BLOCKBUSTER, GRINDHOUSE, BIOPIC, SPAGHETTI_WESTERN, SLASHER, HARDBOILED, VERITE, EPIC, SCREWBALL, ROM_COM, NOIR, PROPAGANDA, MELODRAMA, NEW_WAVE · Scatter: BUBBLEGUM, WITCH_HOUSE, SHOEGAZE, HONKY_TONK, SCREAMO, DRONE, DOO_WOP, SPOKEN_WORD, VAPORWAVE, OI (any case, spaces ok)"))
                        .f("direction", enumeration(&["FORWARD", "BACKWARD"]).optional().describe("Stretch brushes"))
                        .f("gap", number().min(0.25).optional().describe("Scatter brushes: space between marks"))
                        .f("wiggle", number().min(0.0).optional().describe("Scatter"))
                        .f("sizeJitter", number().min(0.0).max(3.0).optional().describe("Scatter"))
                        .f("angularJitter", number().min(-180.0).max(180.0).optional().describe("Scatter, degrees"))
                        .f("rotation", number().min(-180.0).max(180.0).optional().describe("Scatter, degrees"))
                        .build(),
                )
                .optional()
                .describe("Figma's built-in brushes (Stroke settings → Brush); custom brushes can't be set"),
            )
            .build(),
    );

    d.fwd(
        "set_corner_radius",
        "Set corner radius (all corners or individual).",
        shape().f("nodeId", node_id()).spread(&corner_props()).build(),
    );

    d.fwd(
        "set_effects",
        "Replace a node's effects: shadows, layer blur, background blur. Pass an empty shadows list to clear.",
        shape()
            .f("nodeId", node_id())
            .f(
                "shadows",
                array(object(
                    shape()
                        .f("color", hex().default(json!("#0000001A")))
                        .f("x", number().default(json!(0)))
                        .f("y", number().default(json!(4)))
                        .f("blur", px().default(json!(12)))
                        .f("spread", number().default(json!(0)))
                        .f("inner", boolean().default(json!(false)))
                        .build(),
                ))
                .default(json!([])),
            )
            .f("layerBlur", px().optional())
            .f("backgroundBlur", px().optional())
            .build(),
    );

    d.fwd(
        "set_props",
        "Set name, visibility, lock, opacity, clip content, absolute positioning inside auto layout, or rotation.",
        shape()
            .f("nodeId", node_id())
            .f("name", string().optional())
            .f("visible", boolean().optional())
            .f("locked", boolean().optional())
            .f("opacity", number().min(0.0).max(1.0).optional())
            .f("clip", boolean().optional())
            .f("absolute", boolean().optional().describe("Take the node out of its auto-layout flow"))
            .f(
                "rotation",
                number().min(-360.0).max(360.0).optional().describe(
                    "Degrees like Figma's rotation field (positive = counter-clockwise), around the node's centre; 0 = upright. Inside auto layout the parent makes room for the rotated box",
                ),
            )
            .f("allowFlowIssues", allow_flow_issues())
            .build(),
    );

    d.fwd(
        "set_auto_layout",
        "Add or change auto layout on a frame/component (direction, gap, padding, alignment, wrap).",
        shape().f("nodeId", node_id()).spread(&layout_props()).spread(&sizing_props()).build(),
    );

    d.fwd(
        "set_sizing",
        "Set how a node sizes inside auto layout (FIXED/HUG/FILL) and its min/max.",
        shape().f("nodeId", node_id()).spread(&sizing_props()).build(),
    );

    d.fwd(
        "move_resize",
        "Move and/or resize a node. Inside auto layout, prefer set_sizing instead.",
        shape()
            .f("nodeId", node_id())
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().optional())
            .f("height", number().positive().optional())
            .build(),
    );

    d.fwd(
        "move_to_parent",
        "Move a node into another parent (optionally at a child index — order matters in auto layout).",
        shape().f("nodeId", node_id()).f("parentId", string()).f("index", int().min(0.0).optional()).build(),
    );

    d.fwd(
        "rename_node",
        "Rename a node. Renaming a section to \"· In review\" / \"· Approved\" first checks its prototype (flow start, no dead ends, orphans or broken links) and refuses when it is not wired.",
        shape().f("nodeId", node_id()).f("name", string().min(1.0)).f("allowFlowIssues", allow_flow_issues()).build(),
    );

    d.fwd(
        "delete_node",
        "Delete a node. Only nodes made through wridge can be deleted; anything else is the user's work and needs confirm: true, given only after the user agreed.",
        shape()
            .f("nodeId", node_id())
            .f("confirm", boolean().optional().describe("Only after the user explicitly agreed to delete their own node"))
            .build(),
    );

    d.fwd(
        "select_nodes",
        "Select nodes in Figma and zoom to them — do this at the end so the user sees the result.",
        shape().f("nodeIds", node_ids()).f("zoom", boolean().default(json!(true))).build(),
    );

    // ---- images ----

    d.custom(
        "set_image_fill",
        "Fill a node with an image from a local file path or an http(s) URL. PNG/JPEG/GIF, max 4096×4096 px.",
        shape()
            .f("nodeId", node_id())
            .f("source", string().min(1.0).describe("Absolute local path (e.g. C:/Users/me/pic.jpg) or https:// URL"))
            .f(
                "scaleMode",
                enumeration(&["FILL", "FIT", "CROP", "TILE"])
                    .default(json!("FILL"))
                    .describe("FILL = cover, FIT = contain, TILE = repeat"),
            )
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let source = get_str(&args, "source").unwrap_or_default();
            let (base64, format, bytes) = load_image(&source).await?;
            let result = ctx
                .figma(
                    "set_image_fill",
                    json!({"nodeId": get(&args, "nodeId"), "base64": base64, "scaleMode": get(&args, "scaleMode")}),
                    None,
                )
                .await?;
            let mut out = result.as_object().cloned().unwrap_or_default();
            out.insert("format".into(), format.into());
            out.insert("bytes".into(), bytes.into());
            Ok(text(&Value::Object(out)))
        }),
    );

    d.fwd(
        "copy_fills",
        "Copy all fills (images, colors, gradients) from one node onto other nodes.",
        shape().f("fromNodeId", node_id()).f("toNodeIds", node_ids()).build(),
    );

    // ---- design → code ----

    let project_dir = || string().describe(&project_dir_desc());

    d.custom(
        "get_design_context",
        "Read a frame as code-ready data: flex/grid layout, sizing (fill/hug/px), tokens, component instances with their variant states (hover/disabled…), links from prototype interactions, constraints, text and assets. Pass `breakpoints` (e.g. {base: mobileId, md: tabletId, lg: desktopId}) to get one mobile-first tree with per-breakpoint changes.",
        shape()
            .f("nodeId", node_id().optional().describe("A single frame; or use breakpoints"))
            .f(
                "breakpoints",
                partial_record(&BREAKPOINTS, string()).optional().describe("Frame id per breakpoint, smallest first: base (mobile), sm, md (tablet), lg (desktop), xl"),
            )
            .f("depth", int().min(0.0).max(30.0).default(json!(12)))
            .f("expandInstances", boolean().default(json!(false)).describe("Also describe the insides of component instances"))
            .f(
                "userApproved",
                boolean().optional().describe("Screens must be in a \"<Flow> · Approved\" section; true only when the user said to code this design without that review"),
            )
            .f("projectDir", project_dir().optional())
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let dir = get(&args, "projectDir").cloned();
            let id = get(&args, "nodeId").cloned();
            let breakpoints = get(&args, "breakpoints").and_then(Value::as_object).cloned();
            let rest = rest(&args, &["projectDir", "nodeId", "breakpoints"]);
            let mut context;
            if let Some(bps) = breakpoints.as_ref().filter(|b| !b.is_empty()) {
                let mut contexts = vec![];
                for bp in BREAKPOINTS.iter().filter(|b| truthy(bps.get(**b))) {
                    let mut params = rest.clone();
                    params.insert("nodeId".into(), bps[*bp].clone());
                    let c = ctx.figma("get_design_context", Value::Object(params), None).await?;
                    contexts.push(json!({"bp": bp, "context": c}));
                }
                context = codegen::merge_breakpoints(&contexts)?;
            } else if truthy(id.as_ref()) {
                let mut params = rest.clone();
                params.insert("nodeId".into(), id.clone().unwrap());
                context = ctx.figma("get_design_context", Value::Object(params), None).await?;
            } else {
                return Err(anyhow!("Pass nodeId or breakpoints"));
            }
            if truthy(dir.as_ref()) {
                // context.unmappedComponents = annotateComponents(context.root, await readComponentMap(dir))
                prop(Some(&context), "root")?;
                let map = read_component_map(&js_string(dir.as_ref()))?;
                match &mut context {
                    Value::Object(o) => {
                        let unmapped = match o.get_mut("root") {
                            Some(root) => codegen::annotate_components(root, &map),
                            None => vec![],
                        };
                        o.insert("unmappedComponents".into(), Value::Array(unmapped));
                    }
                    Value::Array(_) => {} // a property JSON.stringify leaves out
                    other => {
                        let kind = match other {
                            Value::String(_) => "string",
                            Value::Bool(_) => "boolean",
                            _ => "number",
                        };
                        return Err(anyhow!("Cannot create property 'unmappedComponents' on {kind} '{}'", js_string(Some(other))));
                    }
                }
            }
            // Figma Motion on this frame: which layers animate and whether they ship as code or Lottie. Left out
            // when the editor has no Motion API (or anything below fails), as the JS try/catch does.
            let target = match id.as_ref().filter(|v| !v.is_null()) {
                Some(v) => Some(v.clone()),
                None => breakpoints.as_ref().and_then(|b| b.get("base").filter(|v| !v.is_null()).or_else(|| b.values().next()).cloned()),
            };
            if let Ok(r) = ctx.figma("motion_read", obj! { "nodeId" => target }, None).await {
                if let (Ok(rows), Value::Object(o)) = (motion_handoff::classify(&r, &mut vec![], "ui"), &mut context) {
                    if !rows.is_empty() {
                        let duration = r.get("timelines").and_then(|t| t.get(0)).and_then(|t| fjs::nullish(t.get("duration")));
                        let layers: Vec<Value> = rows
                            .iter()
                            .map(|x| obj! { "id" => x.id.clone(), "name" => x.name.clone(), "to" => x.to.as_str(), "why" => x.why.as_str(), "props" => strings(&x.props), "span" => fjs::nums(&x.span) })
                            .collect();
                        o.insert(
                            "motion".into(),
                            obj! {
                                "timelineMs" => fjs::round(duration.map_or(0.0, fjs::number) * 1000.0),
                                "layers" => layers,
                                "next" => "motion_check → user approves → motion_tokens / motion_to_code (code) and motion_to_lottie (Lottie)",
                            },
                        );
                    }
                }
            }
            Ok(js_text(&context))
        }),
    );

    d.custom(
        "export_assets",
        "Export icons (SVG) and images (PNG) from Figma into a folder of the code project. platform flutter writes PNGs as 1x + 2.0x/ + 3.0x/ variants. Returns the written file paths.",
        shape()
            .f("nodeIds", node_ids())
            .f("outDir", string().describe("Absolute folder, e.g. C:/code/app/public/assets or C:/code/app/assets/images"))
            .f("platform", enumeration(&["web", "flutter"]).default(json!("web")))
            .f("format", enumeration(&["auto", "svg", "png"]).default(json!("auto")).describe("auto = SVG for icons/vectors, PNG otherwise"))
            .f("scale", number().min(1.0).max(4.0).default(json!(2)).describe("Web PNG scale (2 = retina); ignored for flutter"))
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let out_dir = js_string(get(&args, "outDir"));
            codegen::require_absolute(&out_dir, "outDir")?;
            let flutter = get(&args, "platform") == Some(&json!("flutter"));
            let scale = if flutter { Some(json!(1)) } else { get(&args, "scale").cloned() };
            let items: Vec<Value> = arr(get(&args, "nodeIds"))
                .into_iter()
                .map(|id| obj! { "nodeId" => id, "format" => get(&args, "format").cloned(), "scale" => scale.clone() })
                .collect();
            let exported = match ctx.figma("export_assets", json!({ "items": items }), None).await? {
                Value::Array(a) => a,
                Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
                _ => return Err(anyhow!("exported is not iterable")),
            };
            let mut used: HashSet<String> = HashSet::new();
            let mut files: Vec<Value> = vec![];
            for asset in &exported {
                let named = kebab_of(prop(Some(asset), "name")?)?.replace('-', if flutter { "_" } else { "-" });
                let base = if named.is_empty() { format!("asset_{}", kebab_of(prop(Some(asset), "nodeId")?)?) } else { named };
                let format = fjs::to_string(prop(Some(asset), "format")?);
                let mut name = base.clone();
                let mut n = 2;
                while used.contains(&format!("{name}.{format}")) {
                    name = format!("{base}_{n}");
                    n += 1;
                }
                used.insert(format!("{name}.{format}"));
                write_asset(Some(asset), &name, &out_dir, &mut files)?;
                if flutter && prop(Some(asset), "format")? == Some(&json!("png")) {
                    // Flutter picks resolution variants from sibling 2.0x/ and 3.0x/ folders.
                    for s in [2, 3] {
                        let items = json!([obj! { "nodeId" => prop(Some(asset), "nodeId")?.cloned(), "format" => "png", "scale" => s as i64 }]);
                        let variant = first_of(ctx.figma("export_assets", json!({ "items": items }), None).await?)?;
                        write_asset(variant.as_ref(), &name, &node_path::join(&[&out_dir, &format!("{s}.0x")]), &mut files)?;
                    }
                }
            }
            Ok(js_text(&Value::Array(files)))
        }),
    );

    d.custom(
        "export_pdf",
        "Download a document/book design as ONE PDF: each frame in the section is a page (vector, text stays selectable), stacked in order. Order = the number at the start of the frame name (1, 2, 10, 100 — compared as numbers, no zero-padding needed); frames without a number follow by position; hidden frames are skipped. A single frame id gives a one-page PDF. No page numbers are added.",
        shape()
            .f("nodeId", node_id().describe("Section whose frames are the pages (or one frame)"))
            .f("out", string().describe("Absolute .pdf path, or an absolute folder (the file is named after the section)"))
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let out = js_string(get(&args, "out"));
            codegen::require_absolute(&out, "out")?;
            let id = get(&args, "nodeId").cloned().unwrap_or(Value::Null);
            let made = pdf::design_pdf(|command: String, params: Value, timeout: Option<u64>| ctx.figma(&command, params, timeout), &id).await?;
            let file = if out.to_lowercase().ends_with(".pdf") {
                out.clone()
            } else {
                node_path::join(&[&out, &pdf::pdf_file_name(made.title.clone().map(Value::String).as_ref())])
            };
            crate::util::write_file_deep(&file, &made.pdf)?;
            let pages: Vec<Value> = made.pages.iter().enumerate().map(|(i, p)| Value::String(format!("{}. {}", i + 1, fjs::to_string(p.get("name"))))).collect();
            let warnings: Vec<Value> = made.warnings.iter().map(|w| Value::String(w.clone())).collect();
            Ok(js_text(&obj! { "file" => file.replace('\\', "/"), "pages" => pages, "bytes" => made.pdf.len(), "warnings" => warnings }))
        }),
    );

    d.custom(
        "get_tokens",
        "Build a token file (Tailwind v4 @theme, CSS variables, Flutter Dart constants, or JSON) from the file's Foundations — a page, or a section/frame on any page (nested too), named \"Foundations\" (layers named as tokens — works on the free plan) and/or its variables and styles. Returns the content and a map from design-context token names to code identifiers; writes it when outFile is given.",
        shape()
            .f("format", enumeration(&["tailwind", "css", "dart", "json"]).default(json!("tailwind")).describe("tailwind = Tailwind v4 @theme, css = CSS variables, dart = Flutter constants"))
            .f("source", enumeration(&["auto", "foundations", "file"]).default(json!("auto")).describe("auto = Foundations + variables/styles"))
            .f("outFile", string().optional().describe("Absolute path to write, e.g. C:/code/app/app/tokens.css"))
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let data = ctx.figma("get_tokens", obj! { "source" => get(&args, "source").cloned() }, None).await?;
            let built = codegen::format_tokens(&data, &js_string(get(&args, "format")));
            let content = built.get("content").cloned().unwrap_or(Value::Null);
            let out_file = get(&args, "outFile");
            if truthy(out_file) {
                let file = codegen::require_absolute(&js_string(out_file), "outFile")?;
                codegen::write_file_deep(&file, &fjs::to_string(Some(&content)))?;
            }
            Ok(js_text(&obj! {
                "sources" => prop(Some(&data), "sources")?.cloned(),
                "written" => out_file.cloned().unwrap_or(Value::Null),
                "map" => built.get("map").cloned(),
                "content" => content,
            }))
        }),
    );

    d.fwd(
        "scan_design_values",
        "List the colors, typography, radii, gaps, paddings and shadows actually used inside a frame, most frequent first — use it to name tokens / build a Foundations page for a file that has none.",
        shape().f("nodeId", node_id()).f("limit", int().min(1.0).max(50.0).default(json!(12))).build(),
    );

    d.custom(
        "get_component_map",
        &format!("Read {COMPONENT_MAP_FILE}: which Figma components map to which code components (or CSS classes)."),
        shape().f("projectDir", project_dir()).build(),
        handler(|_ctx: Ctx, args: Value| async move { Ok(js_text(&read_component_map(&js_string(get(&args, "projectDir")))?)) }),
    );

    d.custom(
        "map_component",
        &format!("Record (or remove) a Figma component → code mapping in {COMPONENT_MAP_FILE} so future code generation reuses it. Either a code component (import + component) or, for projects whose UI is shared CSS classes and no component exists yet, an element + className (e.g. \"Button/Type=Primary\" → <button className=\"btn btn-p\">). New projects always use components."),
        shape()
            .f("projectDir", project_dir())
            .f("figmaName", string().describe("Component (set) name as in Figma, e.g. \"Button\" or \"Button/Type=Primary\""))
            .f("import", string().optional().describe("Module path, e.g. \"@/components/ui/button\""))
            .f("component", string().optional().describe("Export name, e.g. \"Button\""))
            .f("element", string().optional().describe("Class mapping: HTML element, e.g. \"button\""))
            .f("className", string().optional().describe("Class mapping: base classes, e.g. \"btn\""))
            .f(
                "variants",
                record(string()).optional().describe(
                    "Class mapping: extra classes per variant value, e.g. {\"Type=Primary\": \"btn-p\", \"Type=Danger\": \"btn-d\", \"Size=Small\": \"btn-sm\"}; map the whole set once under its set name",
                ),
            )
            .f("props", record(unknown()).optional().describe("Notes on how Figma props map to code props / extra classes"))
            .f("remove", boolean().default(json!(false)))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let dir = js_string(get(&args, "projectDir"));
            let figma_name = js_string(get(&args, "figmaName"));
            let mut map = match read_component_map(&dir)? {
                Value::Object(o) => o,
                // JSON arrays are objects with index keys to JS
                Value::Array(a) => a.into_iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
                Value::Null if truthy(get(&args, "remove")) => return Err(anyhow!("Cannot convert undefined or null to object")),
                Value::Null => return Err(anyhow!("Cannot set properties of null (setting '{figma_name}')")),
                // delete on a primitive is a no-op; its entries are a string's characters, or none
                Value::String(s) if truthy(get(&args, "remove")) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::String(c.to_string()))).collect(),
                _ if truthy(get(&args, "remove")) => Map::new(),
                other => {
                    let kind = match other {
                        Value::String(_) => "string",
                        Value::Bool(_) => "boolean",
                        _ => "number",
                    };
                    return Err(anyhow!("Cannot create property '{figma_name}' on {kind} '{}'", js_string(Some(&other))));
                }
            };
            if truthy(get(&args, "remove")) {
                map.shift_remove(&figma_name);
            } else {
                let entry = rest(&args, &["projectDir", "figmaName", "remove"]);
                let is_component = truthy(entry.get("import")) && truthy(entry.get("component"));
                if !is_component && !truthy(entry.get("className")) {
                    return Err(anyhow!("Pass import + component, or className (+ element), unless remove is true"));
                }
                if is_component && truthy(entry.get("className")) {
                    return Err(anyhow!("Pass either import + component or className, not both"));
                }
                map.insert(figma_name, Value::Object(entry));
            }
            let entries = map.len();
            let file = codegen::write_component_map(&dir, &Value::Object(map))?;
            Ok(js_text(&obj! { "file" => file.replace('\\', "/"), "entries" => entries }))
        }),
    );

    // ---- code graph (runs in the server, no Figma needed) ----

    let code_dir = || string().describe("Absolute path of the code project");

    d.custom(
        "code_graph",
        "Ask the code graph of a project instead of opening files, and before writing code: what exists to reuse (find also searches tables/columns) and what a change affects (impact). overview | find <text> | file <path> | used_by <target> | uses <target> | impact <target> | route </url or ScreenWidget> | design (Figma components ↔ code) | tokens [token] (design tokens: where used, unused, raw colors, differs from Figma) | notes (AI-written notes: stale ones, and where a note is missing) | data (tables/views/functions from migrations, SQL files or schema strings in code; RLS, who reads/writes them, drift) | table <name> (columns, indexes, relations, policies, every query using it) | flows [name] (agreed behaviour from docs/flows/, ⚠ steps whose code is gone, and drafts for entry points no flow covers — read the flow before exploring code) | trace <route or symbol> (live: calls, tables read/written, packages from there). Target = symbol name (Button), path (components/ui/button.tsx), path#Name, route, or for impact a table / table.column (orders.status). Updates itself for changed files first.",
        shape()
            .f("projectDir", code_dir())
            .f("query", enumeration(&QUERIES))
            .f("target", string().optional())
            .f("depth", int().min(1.0).max(8.0).optional().describe("impact: reverse-dependency levels (default 3); route: render-tree depth (default 3)"))
            .f("limit", int().min(5.0).max(500.0).default(json!(40)))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let dir = js_string(args.get("projectDir"));
            let query = js_string(args.get("query"));
            let target = opt_str(&args, "target");
            let depth = args.get("depth").and_then(Value::as_f64).map(|d| d as i64);
            let limit = args.get("limit").and_then(Value::as_f64).map(|l| l as i64).unwrap_or(40);
            blocking(move || {
                let (db, info) = wg::open_graph(&dir)?;
                let head = codegraph::gitignore_notice(&info);
                let opts = wg::query::QueryOptions { depth, limit, root: Some(dir.clone()) };
                Ok(text_result(head + &wg::query::run_query(&db, &query, target.as_deref(), &opts)?))
            })
            .await
        }),
    );

    d.custom(
        "code_graph_update",
        "Build or refresh <projectDir>/.wara/code-graph.db (adds .wara/ to .gitignore first). code_graph already refreshes itself; use this for the first build, after big changes, or force=true to re-parse everything.",
        shape().f("projectDir", code_dir()).f("force", boolean().default(json!(false))).build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let dir = js_string(args.get("projectDir"));
            let force = truthy(args.get("force"));
            blocking(move || Ok(graph_text(&wg::update_graph(&dir, wg::UpdateOptions { force })?))).await
        }),
    );

    d.custom(
        "code_graph_note",
        "Save what a folder, file or symbol is FOR (purpose, rules, caveats) — 1-3 sentences, not a summary of the code. Target: dir:<folder> | <file path> | path#Name | unique symbol name. Empty text deletes. Notes show up in code_graph answers and are flagged stale when that code changes.",
        shape().f("projectDir", code_dir()).f("target", string()).f("text", string().max(1200.0)).build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let dir = js_string(args.get("projectDir"));
            let target = js_string(args.get("target"));
            let note = js_string(args.get("text"));
            blocking(move || {
                let (db, _) = wg::open_graph(&dir)?;
                Ok(graph_text(&wg::notes::write_note(&db, &wg::query::graph(&db)?, &target, Some(&note))?))
            })
            .await
        }),
    );

    // ---- Figma Motion (beta) ----

    d.custom(
        "motion_read",
        "Figma Motion: read the timeline animation of a frame/layer (selection when nodeId is omitted) and everything inside it — per layer, each animated field (OPACITY, TRANSLATION_X/Y, SCALE_X/Y, ROTATION, PATH_TRIM_END, WIDTH, fills[0] colour…) with its keyframes (seconds), easing (EASE_OUT, BOUNCY, cubic(…), HOLD…) and applied animation styles. styles: true also lists the Figma animation styles (presets) the file offers.",
        shape().f("nodeId", string().optional()).f("styles", boolean().default(json!(false))).build(),
        handler(|ctx: Ctx, args: Value| async move {
            let r = ctx.figma("motion_read", args, None).await?;
            Ok(text_result(motion_figma::motion_text(&r)?))
        }),
    );

    let motion_track = object(
        shape()
            .f("layer", string().default(json!("all")))
            .f("prop", enumeration(&MOTION_PROPS))
            .f("from", any().optional())
            .f("to", any().optional())
            .f("start", number().min(0.0).optional())
            .f("end", number().min(0.0).optional())
            .f("duration", number().min(0.0).optional())
            .f(
                "ease",
                union(vec![string(), array(number()).length(4)])
                    .optional()
                    .describe(&motion_figma::motion_eases().join(" | ")),
            )
            .f("stagger", number().min(0.0).optional())
            .f("keys", array(array(any()).min(2.0).max(3.0)).optional())
            .build(),
    );

    d.custom(
        "motion_animate",
        "Figma Motion: animate layers inside a frame on its timeline — keyframe tracks in the same short language as create_lottie (layer by name / glob / \"all\" / \"root\", prop, from/to, start + end or duration in ms, ease, stagger ms between matched layers, or keys [[ms, value, ease?]]) and/or Figma's own presets (Opacity, Position, Scale, Rotation, Size, Path, TextStagger; motion_read styles:true lists their props). The designer can then tweak it on the timeline. The result is read back. Load the motion skill first (code vs Lottie, motion tokens, taste).",
        shape()
            .f("nodeId", string().describe("The frame whose timeline you animate"))
            .f("duration", number().positive().optional().describe("Timeline length in ms"))
            .f("replace", boolean().default(json!(false)).describe("Remove existing manual tracks and styles on the targeted layers first"))
            .f("tracks", array(motion_track).default(json!([])))
            .f(
                "presets",
                array(object(
                    shape()
                        .f("layer", string().default(json!("all")))
                        .f("style", string())
                        .f("props", record(any()).optional())
                        .f("delay", number().min(0.0).optional())
                        .f("duration", number().positive().optional())
                        .f("stagger", number().min(0.0).optional())
                        .build(),
                ))
                .default(json!([])),
            )
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let id = get(&args, "nodeId").cloned();
            let mut items = motion_figma::tracks_to_items(&arr(get(&args, "tracks")))?;
            let presets = arr(get(&args, "presets"));
            if !presets.is_empty() {
                let r = ctx.figma("motion_read", obj! { "nodeId" => id.clone(), "styles" => true }, None).await?;
                if r.is_null() {
                    return Err(anyhow!("Cannot destructure property 'animationStyles' of '(intermediate value)' as it is null."));
                }
                let available = arr(fjs::nullish(r.get("animationStyles")));
                items.extend(motion_figma::presets_to_items(&presets, &available)?);
            }
            let duration = get(&args, "duration").filter(|d| truthy(Some(d))).map(|d| fjs::number(d) / 1000.0);
            let params = obj! { "rootId" => id.clone(), "items" => items, "replace" => get(&args, "replace").cloned(), "duration" => duration };
            let out = ctx.figma("motion_animate", params, None).await?;
            let o = Some(&out);
            let count = |v: Option<&Value>| length(v).map(|n| n.map_or("undefined".to_string(), |n| n.to_string()));
            let join = |v: Option<&Value>, what: &str, sep: &str| match v {
                Some(Value::Array(a)) => Ok(fjs::join(a, sep)),
                None | Some(Value::Null) => Err(crate::jsjson::read_error(v, "join")),
                _ => Err(anyhow!("out.{what}.join is not a function")),
            };
            let mut lines = vec![format!("Animated {} track(s)/style(s) under {}.", count(prop(o, "applied")?)?, js_string(id.as_ref()))];
            let missing = prop(o, "missing")?;
            if length(missing)?.is_some_and(|n| n > 0) {
                lines.push(format!("⚠ no layer matched: {} (layers: {})", join(missing, "missing", ", ")?, join(prop(o, "layers")?, "layers", ", ")?));
            }
            let id_changes = prop(o, "idChanges")?;
            if fjs::nullish(id_changes).is_some() && length(id_changes)?.is_some_and(|n| n > 0) {
                lines.push(format!(
                    "⚠ Figma gave the frame's layers new ids {} — re-read ids (get_node) before using old ones",
                    join(id_changes, "idChanges", "; ")?
                ));
            }
            lines.push(String::new());
            let after = ctx.figma("motion_read", obj! { "nodeId" => id }, None).await?;
            lines.push(motion_figma::motion_text(&after)?);
            Ok(text_result(lines.join("\n")))
        }),
    );

    let context_param = || {
        enumeration(&["auto", "ui", "landing"]).default(json!("auto")).describe(
            "ui = screens in an app; landing = landing / product / brand pages (long expressive moments, art is part of a code-driven scroll scene). auto reads the frame's section name (Storyboard / landing → landing)",
        )
    };

    d.custom(
        "motion_check",
        "Figma Motion handoff, step 1: for a frame (screen or illustration) list every animated layer and decide code + motion tokens (UI: text, components, auto-layout, images, simple transforms) or Lottie (vector art with path drawing / colour / multi-step motion, and Lottie/… components), plus a lint of the UI motion (too slow, width/height, too many durations/easings, mixed frames). Show the table to the user and get approval before motion_tokens / motion_to_code / motion_to_lottie.",
        shape().f("nodeId", string().describe("Frame with a Figma Motion timeline")).f("context", context_param()).build(),
        handler(|ctx: Ctx, args: Value| async move {
            let id = get(&args, "nodeId").cloned().unwrap_or(Value::Null);
            let h = handoff(&ctx, &[id], &js_string(get(&args, "context"))).await?;
            let read = &h.reads[0];
            let lints = motion_handoff::lint(&h.rows, read, &h.context);
            let mut lines = vec![motion_handoff::check_text(read, &h.rows, &lints, &h.context)];
            lines.extend(h.warnings.iter().map(|w| format!("⚠ {w}")));
            Ok(text_result(lines.join("\n")))
        }),
    );

    d.custom(
        "motion_tokens",
        "Figma Motion handoff: the motion tokens (durations + easings) used by the code-bound animations of one or more frames, as CSS variables and JSON. Run it over all the screens of a flow so they share one set; if the project already has motion tokens, map onto those instead of adding new ones (motion skill).",
        shape().f("nodeIds", array(string()).min(1.0).describe("Frames with Figma Motion timelines")).f("context", context_param()).build(),
        handler(|ctx: Ctx, args: Value| async move {
            let h = handoff(&ctx, &arr(get(&args, "nodeIds")), &js_string(get(&args, "context"))).await?;
            let tk = motion_handoff::tokens(&h.rows);
            if tk.duration.is_empty() {
                return Ok(text_result("No code-bound animation in these frames (motion_check shows what goes to Lottie).".into()));
            }
            let mut lines = vec!["## CSS".to_string(), tk.css, String::new(), "## JSON (tokens file / Figma variables)".into(), tk.json];
            lines.extend(h.warnings.iter().map(|w| format!("⚠ {w}")));
            Ok(text_result(lines.join("\n")))
        }),
    );

    d.custom(
        "motion_to_code",
        "Figma Motion handoff: code for the code-bound animations of a frame (after motion_check is approved) — CSS @keyframes using the motion token variables, or Framer Motion elements with exact per-prop key times and easings. Reduced motion is handled. Adapt the snippet to the project's components; Lottie layers are left to motion_to_lottie.",
        shape()
            .f("nodeId", string())
            .f("target", enumeration(&["css", "framer-motion"]).default(json!("css")))
            .f("tokenFrames", array(string()).optional().describe("Frames whose motion shares the token set (default: this one)"))
            .f("context", context_param())
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let id = get(&args, "nodeId").cloned().unwrap_or(Value::Null);
            let h = handoff(&ctx, &[id], &js_string(get(&args, "context"))).await?;
            let token_frames = arr(get(&args, "tokenFrames"));
            let tk = if token_frames.is_empty() {
                motion_handoff::tokens(&h.rows)
            } else {
                motion_handoff::tokens(&handoff(&ctx, &token_frames, &h.context).await?.rows)
            };
            if !h.rows.iter().any(|x| x.to == "code") {
                return Ok(text_result("Nothing here goes to code — every animated layer is Lottie (motion_to_lottie).".into()));
            }
            let css = get(&args, "target") == Some(&json!("css"));
            let mut warnings = h.warnings;
            let code = if css { motion_handoff::to_css(&h.rows, &tk, &mut warnings) } else { motion_handoff::to_framer(&h.rows, &tk) };
            let skipped: Vec<Value> = h.rows.iter().filter(|x| x.to == "lottie").map(|x| x.name.clone().unwrap_or(Value::Null)).collect();
            let mut lines = vec![format!("```{}", if css { "css" } else { "jsx" }), code, "```".into()];
            if !skipped.is_empty() {
                lines.push(format!("Lottie (not in this code): {}", fjs::join(&skipped, ", ")));
            }
            lines.extend(warnings.iter().map(|w| format!("⚠ {w}")));
            Ok(text_result(lines.join("\n")))
        }),
    );

    // ---- FigJam (system design boards) ----

    let node_kinds: Vec<&str> = NODE_KINDS.iter().copied().filter(|k| !["question", "risk", "agreed", "note"].contains(k)).collect();
    d.custom(
        "figjam_diagram",
        "FigJam: draw a whole system-design diagram in one call (type flow | state | er | sequence) — you give boxes and arrows (or tables, or participants + messages), the server lays them out (lanes = who does each step, columns = order) inside a new section placed clear of existing work. Kinds: start, end, step, wait (⏳ waiting on someone/something), decision (diamond; label its outgoing edges), state (for state machines: edges = transitions), table (data). status: existing (grey, already in the product), new (blue, default), changed (amber), removed (red). notes become stickies beside it: question (yellow), risk (red), agreed (green), note (grey). Loops (retry, go back) are drawn dashed. Load the system-design skill first.",
        shape()
            .f("title", string().describe("Section name, \"<feature> · System design · Draft\""))
            .f(
                "type",
                enumeration(&DIAGRAM_TYPES).default(json!("flow")).describe(
                    "flow: activity diagram with lanes · state: state machine (nodes default to kind state, edges = transitions with the event as label) · er: tables → arrows from FK to referenced table · sequence: participants + time-ordered messages",
                ),
            )
            .f(
                "tables",
                array(object(
                    shape()
                        .f("id", string().optional())
                        .f("name", string())
                        .f("status", enumeration(&NODE_STATUS).optional())
                        .f(
                            "columns",
                            array(object(
                                shape()
                                    .f("name", string())
                                    .f("type", string().optional())
                                    .f("pk", boolean().optional())
                                    .f("fk", string().optional().describe("table.column it references"))
                                    .f("note", string().optional())
                                    .build(),
                            )),
                        )
                        .build(),
                ))
                .optional()
                .describe("er only"),
            )
            .f("participants", array(string()).optional().describe("sequence only, left to right"))
            .f(
                "messages",
                array(object(
                    shape()
                        .f("from", string())
                        .f("to", string())
                        .f("text", string().max(80.0))
                        .f("kind", enumeration(&["call", "reply", "async"]).optional())
                        .build(),
                ))
                .optional()
                .describe("sequence only, in time order; reply = dashed"),
            )
            .f("lanes", array(string()).optional().describe("Actors top to bottom, e.g. ร้านค้า, หน้าเว็บ, server, Slip2Go, LINE OA"))
            .f(
                "nodes",
                array(object(
                    shape()
                        .f("id", string())
                        .f("text", string().max(140.0).default(json!("")).describe("Empty for start / end"))
                        .f("kind", enumeration(&node_kinds).optional())
                        .f("lane", string().optional())
                        .f("status", enumeration(&NODE_STATUS).optional())
                        .build(),
                ))
                .default(json!([]))
                .describe("flow / state"),
            )
            .f(
                "edges",
                array(object(
                    shape().f("from", string()).f("to", string()).f("label", string().max(60.0).optional()).f("dashed", boolean().optional()).build(),
                ))
                .default(json!([])),
            )
            .f(
                "notes",
                array(object(
                    shape().f("text", string().max(300.0)).f("kind", enumeration(&["question", "risk", "agreed", "note"]).default(json!("note"))).build(),
                ))
                .default(json!([])),
            )
            .f("x", number().optional())
            .f("y", number().optional())
            .build(),
        handler(|ctx: Ctx, spec: Value| async move {
            // ELK runs synchronously inside QuickJS: off the async workers
            let draw = tokio::task::spawn_blocking(move || figjam::layout_diagram(&spec)).await.map_err(|e| anyhow!("{e}"))??;
            // FigJam draws slowly (a few seconds per dozen connectors): give it time instead of the usual 30 s.
            let drawn = ctx.figma("figjam_draw", draw.clone(), Some(180_000)).await?;
            if drawn.is_null() {
                return Err(anyhow!("Cannot destructure property 'ids' of '(intermediate value)' as it is null."));
            }
            let section = drawn.get("section");
            let count = |k: &str| draw.get(k).and_then(Value::as_array).map_or(0, Vec::len);
            let lanes = arr(draw.get("lanes"));
            let lanes = if lanes.is_empty() {
                String::new()
            } else {
                let names: Vec<Value> = lanes.iter().map(|l| l.get("name").cloned().unwrap_or(Value::Null)).collect();
                format!(", lanes: {}", fjs::join(&names, " | "))
            };
            let ids: Vec<(String, Value)> = match drawn.get("ids") {
                None | Some(Value::Null) => return Err(anyhow!("Cannot convert undefined or null to object")),
                Some(Value::Object(o)) => fjs::entries(o).into_iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Some(Value::Array(a)) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
                Some(Value::String(s)) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::String(c.to_string()))).collect(),
                Some(_) => vec![],
            };
            let ids: Vec<String> = ids.into_iter().filter(|(k, _)| k != "$section").map(|(k, v)| format!("{k}={}", fjs::to_string(Some(&v)))).collect();
            Ok(text_result(format!(
                "Drew \"{}\" ({}) · {} items, {} arrows{lanes}.\nNode ids → FigJam ids: {}\nRead edits back later with board_map.",
                fjs::to_string(fjs::get(section, "name")),
                fjs::to_string(fjs::get(section, "id")),
                count("nodes"),
                count("edges"),
                ids.join(", ")
            )))
        }),
    );

    d.custom(
        "board_map",
        "FigJam: read a board (or one section) back as compact text — every step/sticky with its lane and text, every arrow as from → to [label], using the ids given in figjam_diagram. Use it after the user edited the board, before writing the flow file.",
        shape().f("sectionId", string().optional()).build(),
        handler(|ctx: Ctx, args: Value| async move {
            let map = ctx.figma("board_map", args, None).await?;
            Ok(text_result(figjam::board_text(&map)?))
        }),
    );

    d.fwd(
        "create_sticky",
        "FigJam: one sticky note (color hex, wide for a wide sticky).",
        shape()
            .f("text", string())
            .f("x", number().optional())
            .f("y", number().optional())
            .f("color", hex().optional())
            .f("wide", boolean().optional())
            .f("parentId", string().optional())
            .f("name", string().optional())
            .build(),
    );

    d.fwd(
        "create_shape",
        "FigJam: one shape with text (square, ellipse, rounded rectangle, diamond…). For whole diagrams use figjam_diagram.",
        shape()
            .f("text", string())
            .f(
                "shape",
                enumeration(&[
                    "SQUARE",
                    "ELLIPSE",
                    "ROUNDED_RECTANGLE",
                    "DIAMOND",
                    "TRIANGLE_UP",
                    "TRIANGLE_DOWN",
                    "PARALLELOGRAM_RIGHT",
                    "PARALLELOGRAM_LEFT",
                    "ENG_DATABASE",
                    "ENG_QUEUE",
                    "ENG_FILE",
                    "ENG_FOLDER",
                ])
                .default(json!("ROUNDED_RECTANGLE")),
            )
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().optional())
            .f("height", number().positive().optional())
            .f("fill", hex().optional())
            .f("stroke", hex().optional())
            .f("textColor", hex().optional())
            .f("parentId", string().optional())
            .f("name", string().optional())
            .build(),
    );

    d.fwd(
        "create_connector",
        "FigJam: an arrow between two existing nodes (it follows them when they move).",
        shape()
            .f("from", string())
            .f("to", string())
            .f("label", string().optional())
            .f("dashed", boolean().optional())
            .f("color", hex().optional())
            .f("lineType", enumeration(&["ELBOWED", "STRAIGHT", "CURVED"]).optional())
            .f("parentId", string().optional())
            .build(),
    );

    d.custom(
        "create_lottie",
        "Make a Lottie animation (splash/logo, success check, loading, empty state, animated icon) from vector art + a short motion spec; the server writes the keyframes. Art: nodeId (a Figma frame — its top-level layers become the animatable layers, named as in Figma), svg markup, or svgFile. Track: layer (name, glob like 'dot*', or 'all') + prop: opacity 0–1 | scale (1 = 100%, or [x,y]) | move [dx,dy] px | rotate deg | draw 0–1 (strokes draw on) | fill / stroke colour; from/to (default = the resting value), start + end or duration in ms, ease (standard, out, in-out, emphasized, back, spring, bounce, hold, linear, cubic(x1,y1,x2,y2)) — an ease shapes the segment leaving its key, stagger ms between matched layers — or keys [[ms, value, ease?], …]. UI motion (page transitions, sheets, lists, buttons) belongs in code with motion tokens, not Lottie: load the motion skill first.",
        shape()
            .f("projectDir", code_dir())
            .f("out", string().regex("\\.json$", "i").describe("Path inside the project, e.g. assets/lottie/success.json (Flutter) or public/lottie/success.json (web)"))
            .f("nodeId", string().optional().describe("Figma node to take the art from"))
            .f("svg", string().optional())
            .f("svgFile", string().optional().describe("Path of an .svg file (absolute, or inside the project)"))
            .f("name", string().optional())
            .f("width", number().positive().optional().describe("Output size; default = the art's size"))
            .f("height", number().positive().optional())
            .f("duration", number().positive().max(60_000.0).describe("Total length in ms"))
            .f("fps", int().min(12.0).max(120.0).default(json!(60)))
            .f(
                "poster",
                number().min(0.0).optional().describe(
                    "ms of the frame shown when not playing (cards, reduced motion, the Figma GIF's first frame). Default: the last frame — set it for loops that fade out at the end",
                ),
            )
            .f(
                "tracks",
                array(object(
                    shape()
                        .f("layer", string().default(json!("all")))
                        .f("prop", enumeration(&["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"]))
                        .f("from", any().optional())
                        .f("to", any().optional())
                        .f("start", number().min(0.0).optional())
                        .f("end", number().min(0.0).optional())
                        .f("duration", number().min(0.0).optional())
                        .f("ease", union(vec![string(), array(number()).length(4)]).optional())
                        .f("stagger", number().min(0.0).optional())
                        .f("keys", array(array(any()).min(2.0).max(3.0)).optional())
                        .build(),
                ))
                .default(json!([])),
            )
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let spec = rest(&args, &["projectDir", "out", "nodeId", "svg", "svgFile"]);
            write_lottie(
                &ctx,
                LottieJob {
                    args: &args,
                    spec,
                    source: "spec",
                    extra_warnings: vec![],
                },
            )
            .await
        }),
    );

    d.custom(
        "motion_to_lottie",
        "Figma Motion → Lottie: turn the animation a frame has on its Figma Motion timeline (made with motion_animate or by the designer) into a Lottie file in the code project. Art = the frame's SVG; tracks = what motion_read sees on its top-level layers (rotation, scale, move, opacity, path draw, fill/stroke colour; Figma springs become curves). Use it for illustrations / icons that animate as one piece — UI motion goes to code + motion tokens (motion skill). The recipe is kept in meta, so the file can be rebuilt after the design changes.",
        shape()
            .f("projectDir", code_dir())
            .f("out", string().regex("\\.json$", "i").describe("Path inside the project, e.g. public/lottie/success.json"))
            .f("nodeId", string().describe("The frame with the Motion timeline"))
            .f("name", string().optional())
            .f("width", number().positive().optional())
            .f("height", number().positive().optional())
            .f("fps", int().min(12.0).max(120.0).default(json!(60)))
            .f("poster", number().min(0.0).optional().describe("ms of the still frame (default: the last frame)"))
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let read = ctx.figma("motion_read", obj! { "nodeId" => get(&args, "nodeId").cloned() }, None).await?;
            let made = motion_figma::motion_to_spec(&read)?;
            let warnings: Vec<String> = arr(made.get("warnings")).iter().map(|w| js_string(Some(w))).collect();
            let tracks = made.get("tracks").cloned().unwrap_or(Value::Array(vec![]));
            if arr(Some(&tracks)).is_empty() {
                let why = if warnings.is_empty() { "no Motion animation on this frame's layers".to_string() } else { warnings.join("; ") };
                return Err(anyhow!("Nothing to export: {why} — animate it first (motion_animate)"));
            }
            let mut spec = rest(&args, &["projectDir", "out", "nodeId"]);
            spec.insert("duration".into(), made.get("duration").cloned().unwrap_or(Value::Null));
            spec.insert("tracks".into(), tracks);
            if let Some(split) = made.get("split") {
                spec.insert("split".into(), split.clone());
            }
            write_lottie(
                &ctx,
                LottieJob {
                    args: &args,
                    spec,
                    source: "figma-motion",
                    extra_warnings: warnings,
                },
            )
            .await
        }),
    );

    d.custom(
        "code_design_sync",
        "Copy the open Figma file's components and Foundations/variable tokens into the project's code graph (needs the Figma plugin). Then code_graph design / tokens can link them to code even when Figma is closed.",
        shape().f("projectDir", code_dir()).build(),
        handler(|ctx: Ctx, args: Value| async move {
            let dir = js_string(args.get("projectDir"));
            let (db, _) = blocking(move || wg::open_graph(&dir)).await?;
            let info = ctx.figma("get_document_info", json!({}), None).await?;
            let components = ctx.figma("get_components", json!({}), None).await?;
            let data = ctx.figma("get_tokens", json!({"source": "auto"}), Some(120_000)).await?;
            blocking(move || Ok(graph_text(&wg::design::sync_design_from(&db, &info, &components, &data)?))).await
        }),
    );

    d.custom(
        "project_knowledge",
        "Researched guidance for this project's tools, stored in <project>/.wara/knowledge/<topic>/<tool>@<major>.md (gitignored). list: the detected stack (package.json / go.mod versions), project facts, saved notes, stale ones, and with topic the tools still missing a note. get: one note. save: after researching (official docs first; 1–2 searches, ≤ 3 fetched pages — release/advisory pages and the needed guide section, not whole manuals), a short note with its source URLs. fact: store a user's answer about the project (key + value, e.g. deploy, redis, ci; empty value deletes) so it is asked only once. Reuse notes/facts instead of searching or asking again; re-research only when missing or stale.",
        shape()
            .f("projectDir", code_dir())
            .f("action", enumeration(&["list", "get", "save", "fact"]))
            .f("key", string().optional().describe("fact: e.g. \"deploy\", \"redis\", \"queue\", \"ci\""))
            .f("value", string().optional().describe("fact: the user's answer, e.g. \"VPS + Docker Compose, 1 server\""))
            .f("topic", string().optional().describe("e.g. \"security\", \"cache\""))
            .f("tool", string().optional().describe("Package / module as in the stack list, e.g. \"next\", \"github.com/gofiber/fiber\""))
            .f("version", string().optional().describe("save: defaults to the detected version"))
            .f("content", string().max(8000.0).optional().describe("save: the rules for this tool at this version — concrete APIs and settings, not generic advice"))
            .f("sources", array(string().url()).optional().describe("save: URLs the note is based on (at least one)"))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let action = get_str(&args, "action").unwrap_or_default();
            let dir = js_string(get(&args, "projectDir"));
            if action == "list" {
                let topic = opt_str(&args, "topic");
                return blocking(move || Ok(graph_text(&wg::knowledge::list_knowledge(&dir, topic.as_deref())?))).await;
            }
            if action == "fact" {
                if !truthy(get(&args, "key")) || get(&args, "value").is_none() {
                    return Err(anyhow!("fact needs key and value"));
                }
                let (key, value) = (js_string(get(&args, "key")), js_string(get(&args, "value")));
                return blocking(move || Ok(graph_text(&wg::knowledge::set_fact(&dir, &key, &value)?))).await;
            }
            if !truthy(get(&args, "topic")) || !truthy(get(&args, "tool")) {
                return Err(anyhow!("topic and tool are required for get/save"));
            }
            let (topic, tool) = (js_string(get(&args, "topic")), js_string(get(&args, "tool")));
            if action == "get" {
                return blocking(move || Ok(graph_text(&wg::knowledge::get_knowledge(&dir, &topic, &tool)?))).await;
            }
            let sources_empty = get(&args, "sources").and_then(Value::as_array).map(|a| a.is_empty()).unwrap_or(true);
            if !truthy(get(&args, "content")) || sources_empty {
                return Err(anyhow!("save needs content and at least one source URL"));
            }
            let save = wg::knowledge::SaveArgs {
                topic,
                tool,
                version: opt_str(&args, "version"),
                content: js_string(get(&args, "content")),
                sources: opt_strings(&args, "sources"),
            };
            blocking(move || Ok(graph_text(&wg::knowledge::save_knowledge(&dir, &save)?))).await
        }),
    );

    d.custom(
        "tasks",
        "The project's task board, shared with the person (wridge → Tasks; file docs/tasks/board.json). list (filters: status, label, from, query) · get id · add tasks[] · update id + set · move id + status (+ index) · tick id + subtask · note id + text · accept / skip id (+ text = the reason) · remove id. Found something to fix while checking? add it with change {where, old, new} and from \"<check> · <date>\": it waits in Review until the person accepts; after accept make the change, then move it to done. Never raise a skipped one again (list status: skipped shows the reasons). Before first use: get_skill(\"reports-tasks\").",
        shape()
            .f("projectDir", string().describe("Absolute path of the project"))
            .f("action", enumeration(&["list", "get", "add", "update", "move", "tick", "note", "accept", "skip", "remove"]))
            .f("id", string().optional().describe("e.g. \"T-3\""))
            .f(
                "tasks",
                array(object(
                    shape()
                        .f("title", string())
                        .f("description", string().optional().describe("Markdown"))
                        .f("status", enumeration(&["todo", "doing", "review", "done"]).optional().describe("default todo; review when change is given"))
                        .f("labels", array(string()).optional())
                        .f("subtasks", array(string()).optional())
                        .f("links", array(string()).optional().describe("file:line, docs/flows/x.md, figma:12:34, a PR or https URL"))
                        .f("due", string().optional())
                        .f("who", string().optional().describe("who does it: AI or a name"))
                        .f("from", string().optional().describe("the batch, e.g. \"AI check · 7 Nov\""))
                        .f(
                            "change",
                            object(shape().f("where", string().optional()).f("old", string()).f("new", string()).build())
                                .optional()
                                .describe("a suggested edit the person accepts or skips"),
                        )
                        .build(),
                ))
                .optional()
                .describe("add: one or more tasks"),
            )
            .f("set", record(any()).optional().describe("update: the fields to replace (names as in tasks[])"))
            .f("status", enumeration(&["todo", "doing", "review", "done", "skipped", "all"]).optional().describe("move: the new column · list: filter (skipped are hidden unless asked)"))
            .f("index", int().min(0.0).optional().describe("move: position in the column"))
            .f("subtask", int().min(1.0).optional().describe("tick: its number, as get shows"))
            .f("done", boolean().optional().describe("tick: false unticks"))
            .f("text", string().max(2000.0).optional().describe("note: the note · skip: the reason"))
            .f("label", string().optional().describe("list filter"))
            .f("from", string().optional().describe("list filter: the batch name contains this"))
            .f("query", string().optional().describe("list filter: words in title or description"))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let dir = js_string(get(&args, "projectDir"));
            blocking(move || Ok(text_result(crate::tasks::apply(&dir, &args, "AI")?.0))).await
        }),
    );

    d.custom(
        "report",
        "Reports the person reads in wridge → Reports and exports as image or PDF — no Figma needed (docs/reports/<name>.json). write: report {title, subtitle?, eyebrow?, status?, theme?, rows: [block | [block, block] (≤3 side by side)]}; blocks: header, stats, compare, chart (kind line|bars|hbars|stacked|area|donut|pie|scatter|gauge|heatmap|funnel), board, table (preset basic|status|numbers|progress|compare|grouped), timeline, callout, checklist, flow, text, image, tasks (live from the Tasks board: {type: \"tasks\", filter: {from|label|status}}), divider. Send content only — colors, fonts, sizes and layout have defaults; set theme / block style / item colors only when asked. theme: save tokens under name, then \"theme\": \"<name>\". list · get name · remove name · export name + format png|pdf. Load get_skill(\"reports-tasks\") for each block's fields before the first report.",
        shape()
            .f("projectDir", string().describe("Absolute path of the project"))
            .f("action", enumeration(&["list", "get", "write", "remove", "theme", "export"]))
            .f("name", string().optional().describe("file name in docs/reports/ (write: defaults to the title)"))
            .f("report", record(any()).optional().describe("write: {title, rows: [...]} — see the skill for each block"))
            .f("mode", enumeration(&["create", "replace"]).optional().describe("write: default create (fails if the name exists)"))
            .f("theme", record(any()).optional().describe("theme: tokens to save under name, e.g. {\"accent\": \"#7A3BC4\"}"))
            .f("format", enumeration(&["png", "pdf"]).optional().describe("export: default png"))
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let dir = js_string(get(&args, "projectDir"));
            let obj = args.as_object().cloned().unwrap_or_default();
            Ok(text_result(crate::reports::tool(&dir, &obj, ctx.http_port).await?))
        }),
    );

    d.custom(
        "load_test",
        "Load-test the project with k6 (the person gets it once — Download k6 in wridge Settings, or an install; action check tells how; never download it yourself). The script (tests/load/<flow>.js, written per get_skill(\"scale-traffic\")) takes the base URL from __ENV.BASE_URL. run starts it in the background and answers at once with a run id; status shows progress and, once k6 ends, the numbers and a report it wrote to wridge → Reports (add your findings to that report after). stop ends a run. Guarded: only local or staging targets, at most 2000 virtual users and 30 minutes — anything beyond needs confirmed: true after the person agreed. Before every run, tell the person the four limits from the skill.",
        shape()
            .f("projectDir", string().describe("Absolute path of the project"))
            .f("action", enumeration(&["check", "run", "status", "stop"]).default(json!("run")))
            .f("script", string().optional().describe("run: the k6 script, relative to the project, e.g. tests/load/booking.js"))
            .f("target", string().optional().describe("run: base URL the script hits, e.g. http://localhost:3000 (passed as BASE_URL)"))
            .f("run", string().optional().describe("status / stop: the run id (status without one: the latest run)"))
            .f("confirmed", boolean().optional().describe("run: the person agreed to a non-local target or a run above the limits"))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let obj = args.as_object().cloned().unwrap_or_default();
            Ok(text_result(crate::loadtest::tool(&obj).await?))
        }),
    );

    d.custom(
        "project_memory",
        "What was agreed and where work stands, kept outside the chat so compaction or a new session loses nothing — for any project (app, game, book, study…), code or not. overview: open tasks, decision titles, brain tags — start here. find: matching decisions, working notes and brain notes (one line each). decide: a project-level decision the user agreed to → docs/decisions.md. note: a short agreement made while working on a task (.wara). handoff: create/update a task's state; close: true when finished. Brain = the project's knowledge as Markdown files in brain/ that the user reads too (Obsidian-style [[links]]: [[Note]], [[code:path#Name]], [[figma:12:34]], [[roblox:Path]], [[project:Note]]; a front-matter key with links is a named relation, e.g. boss_of: [[CH1]]). read: one whole note + its links and backlinks. write: name + content (mode create | append | replace; replace keeps the old text in the trash). trash: move a note to brain/.trash (restore with path). Only the project of this session; another project only when the user asks. Search before reading; write only what was decided or is hard to find again, not chat trivia. Never open docs/decisions.md or brain/ files whole — use find/read.",
        shape()
            .f("projectDir", string().describe("Absolute path of the project"))
            .f("action", enumeration(&["overview", "find", "decide", "note", "handoff", "read", "write", "trash", "restore"]))
            .f("name", string().optional().describe("read/write/trash: note name (file name, title or alias); a subfolder is allowed, e.g. \"characters/Nathee\""))
            .f("content", string().max(20000.0).optional().describe("write: Markdown; starts with # Title unless front matter is given"))
            .f("mode", enumeration(&["create", "append", "replace"]).optional().describe("write: default create (fails if the note exists)"))
            .f("path", string().optional().describe("restore: the trash path that trash returned"))
            .f("query", string().optional().describe("find: topic words"))
            .f("includeOld", boolean().optional().describe("find: also replaced decisions and closed tasks' notes"))
            .f("task", string().optional().describe("note/handoff: short task name, e.g. \"edupro Foundations in Figma\""))
            .f("title", string().optional().describe("decide: short title"))
            .f("decision", string().optional().describe("decide: what was decided, 1–3 sentences"))
            .f("why", string().optional().describe("decide: the reason"))
            .f("supersedes", string().optional().describe("decide: id it replaces, e.g. \"D-3\""))
            .f("text", string().max(600.0).optional().describe("note: the agreement, 1–2 sentences"))
            .f("tags", array(string()).optional())
            .f("stage", string().optional().describe("handoff: e.g. \"5/8 Prototype\""))
            .f("done", string().optional())
            .f("next", string().optional())
            .f("questions", string().optional().describe("handoff: what waits on the user"))
            .f("close", boolean().optional().describe("handoff: the task is finished"))
            .build(),
        handler(|_ctx: Ctx, args: Value| async move {
            let action = get_str(&args, "action").unwrap_or_default();
            let has = |k: &str| truthy(get(&args, k));
            let dir = js_string(get(&args, "projectDir"));
            let s = |k: &str| js_string(get(&args, k));
            match action.as_str() {
                "overview" => blocking(move || Ok(graph_plain(&Value::String(wg::memory::overview(&dir, false)?)))).await,
                "read" | "write" | "trash" => {
                    if !has("name") {
                        return Err(anyhow!("{action} needs name"));
                    }
                    let name = s("name");
                    match action.as_str() {
                        "read" => blocking(move || Ok(text_result(note_text(&wg::brain::read_note(&dir, &name)?)))).await,
                        "write" => {
                            let opts = wg::brain::WriteOptions {
                                text: opt_str(&args, "content"),
                                mode: opt_str(&args, "mode").unwrap_or_else(|| "create".into()),
                                tags: opt_strings(&args, "tags"),
                                keep_old: true,
                            };
                            blocking(move || Ok(graph_text(&wg::brain::write_note(&dir, &name, &opts)?))).await
                        }
                        _ => blocking(move || Ok(graph_text(&wg::brain::trash_note(&dir, &name)?))).await,
                    }
                }
                "restore" => {
                    if !has("path") {
                        return Err(anyhow!("restore needs path (from trash)"));
                    }
                    let path = s("path");
                    blocking(move || Ok(graph_text(&wg::brain::restore_note(&dir, &path)?))).await
                }
                "find" => {
                    if !has("query") {
                        return Err(anyhow!("find needs query"));
                    }
                    let (query, old) = (s("query"), truthy(get(&args, "includeOld")));
                    blocking(move || Ok(graph_plain(&Value::String(wg::memory::find(&dir, &query, old, 8)?)))).await
                }
                "decide" => {
                    if !has("title") || !has("decision") {
                        return Err(anyhow!("decide needs title and decision"));
                    }
                    let a = wg::memory::DecideArgs {
                        title: s("title"),
                        decision: s("decision"),
                        why: opt_str(&args, "why"),
                        tags: opt_strings(&args, "tags"),
                        supersedes: opt_str(&args, "supersedes"),
                    };
                    blocking(move || Ok(graph_plain(&wg::memory::decide(&dir, &a)?))).await
                }
                "note" => {
                    if !has("text") {
                        return Err(anyhow!("note needs text"));
                    }
                    let (task, text, tags) = (opt_str(&args, "task"), s("text"), opt_strings(&args, "tags"));
                    blocking(move || Ok(graph_plain(&wg::memory::note(&dir, task.as_deref(), &text, tags.as_deref())?))).await
                }
                _ => {
                    if !has("task") {
                        return Err(anyhow!("handoff needs task"));
                    }
                    let a = wg::memory::HandoffArgs {
                        task: s("task"),
                        stage: opt_str(&args, "stage"),
                        done: opt_str(&args, "done"),
                        next: opt_str(&args, "next"),
                        questions: opt_str(&args, "questions"),
                        close: truthy(get(&args, "close")),
                    };
                    blocking(move || Ok(graph_plain(&Value::String(wg::memory::handoff(&dir, &a)?)))).await
                }
            }
        }),
    );

    // ---- design graph ----

    d.graph(
        "design_map",
        "Compact overview of the whole file: pages, Prototype screens only (size, review status from their section name: draft / review / approved, components used, where they link to); Slides / Storyboard / Graphic sections are listed as other work, flows, breakpoint sets, component usage and unused components. Cached and refreshed automatically when the file changes. Start here instead of reading screens one by one.",
        shape()
            .f("pageId", string().optional().describe("Limit to one page (id or name)"))
            .f("refresh", boolean().default(json!(false)))
            .f("format", enumeration(&["text", "json"]).default(json!("text")))
            .f("maxScreens", int().min(1.0).max(1000.0).default(json!(150)).describe("Per page, in text format"))
            .build(),
        |map, args| {
            if args.get("format") == Some(&json!("json")) {
                text(&map)
            } else {
                let max = args.get("maxScreens").and_then(Value::as_f64).unwrap_or(150.0) as usize;
                text_result(render_design_map(&map, max))
            }
        },
    );

    d.graph(
        "component_usage",
        "Where components are used: pass component (name, partial ok) → screens + instance ids per variant; pass screenId → components on that screen; neither → all components by usage.",
        shape().f("component", string().optional()).f("screenId", string().optional().describe("Screen id or name")).f("pageId", string().optional()).build(),
        |v, _| text(&v),
    );

    d.graph(
        "find_duplicates",
        "Find layers that should be one component: identical structures drawn separately (duplicate) and near-copies with different spacing/radius/font sizes (inconsistent). Proposes only — ask the user before converting.",
        shape()
            .f("scopeId", string().optional().describe("Page or frame to scan; defaults to the current page"))
            .f("minCount", int().min(2.0).default(json!(2)))
            .f("limit", int().min(1.0).max(50.0).default(json!(15)))
            .build(),
        |v, _| text(&v),
    );

    d.graph(
        "check_flows",
        "Check prototype flows: dead ends (reached but no way out), orphan screens, screens unreachable from flow starts, broken links, and buttons/tabs/links without any interaction.",
        shape().f("pageId", string().optional()).f("limit", int().min(1.0).max(200.0).default(json!(30))).build(),
        |v, _| text(&v),
    );

    // ---- shapes ----

    let shape_base = || {
        shape()
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().default(json!(100)))
            .f("height", number().positive().default(json!(100)))
            .f("parentId", parent_id())
            .f("fill", hex().optional())
            .spread(&sizing_props())
            .build()
    };

    d.fwd(
        "create_polygon",
        "Create a regular polygon (3 = triangle).",
        shape()
            .f("name", string().default(json!("Polygon")))
            .f("pointCount", int().min(3.0).max(60.0).default(json!(3)))
            .spread(&shape_base())
            .build(),
    );

    d.fwd(
        "create_star",
        "Create a star.",
        shape()
            .f("name", string().default(json!("Star")))
            .f("pointCount", int().min(3.0).max(60.0).default(json!(5)))
            .f("innerRadius", number().min(0.0).max(1.0).default(json!(0.382)).describe("0–1, how deep the points are"))
            .spread(&shape_base())
            .build(),
    );

    d.fwd(
        "create_line",
        "Create a straight line (dividers, connectors). Rotation in degrees.",
        shape()
            .f("name", string().default(json!("Line")))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("length", number().positive().default(json!(100)))
            .f("rotation", number().optional())
            .f("color", hex().default(json!("#000000")))
            .f("weight", px().default(json!(1)))
            .f("cap", enumeration(&["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL"]).optional())
            .f("parentId", parent_id())
            .spread(&sizing_props())
            .build(),
    );

    d.custom(
        "create_from_svg",
        "Import SVG markup (or a local .svg file) as a frame of vectors — the right way to add icons. `color` recolors the icon's shapes (solid fills/strokes, keeping their opacity); the wrapping frame's background is left as is.",
        shape()
            .f("svg", string().optional().describe("SVG markup starting with <svg"))
            .f("file", string().optional().describe("Absolute path to a local .svg file"))
            .f("name", string().default(json!("Icon")))
            .f("x", number().optional())
            .f("y", number().optional())
            .f("width", number().positive().optional().describe("Scale proportionally to this width"))
            .f("color", hex().optional())
            .f("parentId", parent_id())
            .spread(&sizing_props())
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let markup: Option<String> = match get(&args, "svg") {
                Some(Value::String(s)) => Some(s.clone()),
                _ => match get(&args, "file") {
                    Some(Value::String(f)) if !f.is_empty() => Some(String::from_utf8_lossy(&crate::util::read_file(Path::new(f))?).into_owned()),
                    _ => None,
                },
            };
            let markup = match markup {
                Some(m) if !m.is_empty() => m,
                _ => return Err(anyhow!("Pass svg (markup) or file (path to .svg)")),
            };
            if len_utf16(&markup) > 1_000_000 {
                return Err(anyhow!("SVG is larger than 1 MB"));
            }
            if regress::Regex::with_flags("<svg[\\s>]", "i").unwrap().find(&markup).is_none() {
                return Err(anyhow!("Not SVG markup — expected an <svg> element"));
            }
            let mut params = rest(&args, &["svg", "file"]);
            params.insert("svg".into(), Value::String(markup));
            Ok(text(&ctx.figma("create_from_svg", Value::Object(params), None).await?))
        }),
    );

    d.fwd(
        "boolean_shapes",
        "Combine sibling layers with a boolean operation (editable afterwards, like Figma's toolbar). For SUBTRACT the bottom-most layer is the base.",
        shape()
            .f("nodeIds", array(string()).min(2.0))
            .f("operation", enumeration(&["UNION", "SUBTRACT", "INTERSECT", "EXCLUDE"]))
            .f("name", string().optional())
            .build(),
    );

    d.fwd(
        "flatten",
        "Flatten sibling layers into one vector (not reversible — prefer boolean_shapes when editing may be needed).",
        shape().f("nodeIds", node_ids()).f("name", string().optional()).build(),
    );

    d.fwd(
        "outline_stroke",
        "Convert a layer's stroke into a filled vector shape (placed above the original).",
        shape()
            .f("nodeId", node_id())
            .f("removeOriginal", boolean().default(json!(false)))
            .f("name", string().optional())
            .build(),
    );

    d.fwd(
        "vector_read",
        "Read a shape's points like Figma's vector edit mode (Enter): points [{i, x, y}], segments [{i, from, to, curve?}] and closed regions, plus which segments / points lie on each side (top, bottom, left, right) and the corners. Works on vectors, rectangles, ellipses, polygons, stars and lines (read only — nothing changes).",
        shape().f("nodeId", node_id()).build(),
    );

    d.fwd(
        "vector_edit",
        "Edit a shape's points — Figma's vector edit mode without the mouse. A rectangle / ellipse / polygon / star / line becomes a vector first (its id changes: use the returned id). ops run in order: move {points, dx, dy | x, y}; curve {segments, amount} (bend a segment: + bulges outward, - inward, px at the middle); handles {segment, start: [x, y], end: [x, y]}; straighten {segments}; split {segment, at 0–1} (add a point); cut {segment, at} (open the path there); remove {points | segments} (a removed point between two segments joins its neighbours); connect {from, to}; corner {points, radius}; cap {points, cap}; mirror {points, mode}; paint {region, color} (fill one closed area). points / segments: an index from vector_read, a list, a side (\"top\", \"bottom\", \"left\", \"right\"), a corner (\"top-left\"…) or \"all\".",
        shape()
            .f("nodeId", node_id())
            .f(
                "ops",
                array(object(
                    shape()
                        .f("op", enumeration(&["move", "curve", "handles", "straighten", "split", "cut", "remove", "connect", "corner", "cap", "mirror", "paint"]))
                        .f("points", union(vec![int(), string(), array(union(vec![int(), string()]))]).optional())
                        .f("segments", union(vec![int(), string(), array(union(vec![int(), string()]))]).optional())
                        .f("segment", int().min(0.0).optional())
                        .f("region", int().min(0.0).optional())
                        .f("from", int().min(0.0).optional())
                        .f("to", int().min(0.0).optional())
                        .f("dx", number().optional())
                        .f("dy", number().optional())
                        .f("x", number().optional())
                        .f("y", number().optional())
                        .f("amount", number().optional())
                        .f("at", number().min(0.0).max(1.0).optional())
                        .f("start", array(number()).optional())
                        .f("end", array(number()).optional())
                        .f("radius", number().min(0.0).optional())
                        .f("cap", enumeration(&["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL", "DIAMOND_FILLED", "TRIANGLE_FILLED", "CIRCLE_FILLED"]).optional())
                        .f("mode", enumeration(&["NONE", "ANGLE", "ANGLE_AND_LENGTH"]).optional())
                        .f("color", hex_or_none().optional())
                        .build(),
                ))
                .min(1.0),
            )
            .build(),
    );

    d.fwd(
        "bend_shape",
        "Bend a shape in one step (ready-made vector edits): bulge (a side curves out, or in with a negative amount), wave (a side becomes count waves), pinch (a side's ends move toward each other — trapezoid; negative spreads), skew (a side slides along itself), chamfer (cut corners off), round (round chosen corners). side: top | bottom | left | right | all; corner: top-left | top-right | bottom-right | bottom-left | all. Turns rectangles, ellipses, polygons and stars into a vector (new id returned).",
        shape()
            .f("nodeId", node_id())
            .f("effect", enumeration(&["bulge", "wave", "pinch", "skew", "chamfer", "round"]))
            .f("side", enumeration(&["top", "bottom", "left", "right", "all"]).optional().describe("bulge, wave, pinch, skew"))
            .f("corner", enumeration(&["top-left", "top-right", "bottom-right", "bottom-left", "all"]).optional().describe("chamfer, round"))
            .f("amount", number().describe("px: how far it bends / moves / cuts (round: the radius)"))
            .f("count", int().min(1.0).max(20.0).default(json!(3)).describe("wave: number of waves"))
            .build(),
    );

    d.fwd(
        "use_as_mask",
        "Use a layer as a mask for sibling layers (like right-click → Use as mask). Groups them with the mask at the bottom.",
        shape()
            .f("maskNodeId", string().describe("The shape that defines the visible area"))
            .f("nodeIds", node_ids().describe("Layers to be clipped by the mask (same parent as the mask)"))
            .f(
                "maskType",
                enumeration(&["ALPHA", "VECTOR", "LUMINANCE"])
                    .default(json!("ALPHA"))
                    .describe("ALPHA = by opacity, VECTOR = by shape outline, LUMINANCE = by brightness"),
            )
            .f("name", string().optional())
            .build(),
    );

    d.fwd("remove_mask", "Turn a mask layer back into a normal layer.", shape().f("nodeId", node_id()).build());

    // ---- prototype ----

    d.fwd(
        "get_prototype",
        "Read prototype flows (starting points) and every interaction on the current page, or inside `nodeId`.",
        shape().f("nodeId", string().optional()).f("limit", int().min(1.0).max(500.0).default(json!(50))).build(),
    );

    d.custom(
        "add_interaction",
        "Add a prototype interaction (the blue connector): a trigger on `nodeId` that navigates, opens an overlay, goes back, etc.",
        shape()
            .f("nodeId", node_id().describe("The layer the user interacts with (button, tab, card…)"))
            .f(
                "trigger",
                enumeration(&[
                    "ON_CLICK",
                    "ON_HOVER",
                    "ON_PRESS",
                    "ON_DRAG",
                    "AFTER_TIMEOUT",
                    "MOUSE_ENTER",
                    "MOUSE_LEAVE",
                    "MOUSE_DOWN",
                    "MOUSE_UP",
                ])
                .default(json!("ON_CLICK"))
                .describe("AFTER_TIMEOUT only works on top-level frames"),
            )
            .f(
                "delay",
                number()
                    .min(0.0)
                    .optional()
                    .describe("Seconds: the timeout for AFTER_TIMEOUT, or delay for MOUSE_* triggers"),
            )
            .f(
                "action",
                enumeration(&["NAVIGATE", "OVERLAY", "SWAP", "SCROLL_TO", "CHANGE_TO", "BACK", "CLOSE", "URL"])
                    .default(json!("NAVIGATE"))
                    .describe("SWAP = swap overlay, CHANGE_TO = switch component variant, CLOSE = close overlay"),
            )
            .f("destinationId", string().optional().describe("Target frame/layer; required except for BACK, CLOSE and URL"))
            .f("url", string().url().optional().describe("Required for URL"))
            .f(
                "transition",
                enumeration(&["INSTANT", "DISSOLVE", "SMART_ANIMATE", "MOVE_IN", "MOVE_OUT", "PUSH", "SLIDE_IN", "SLIDE_OUT"]).default(json!("SMART_ANIMATE")),
            )
            .f(
                "direction",
                enumeration(&["LEFT", "RIGHT", "TOP", "BOTTOM"])
                    .default(json!("LEFT"))
                    .describe("For MOVE/PUSH/SLIDE: the side it moves toward"),
            )
            .f("duration", number().min(0.0).max(10.0).default(json!(0.3)).describe("Seconds"))
            .f("easing", enumeration(&["EASE_OUT", "EASE_IN", "EASE_IN_AND_OUT", "LINEAR"]).default(json!("EASE_OUT")))
            .f("preserveScroll", boolean().default(json!(false)))
            .f(
                "replace",
                boolean()
                    .default(json!(true))
                    .describe("Replace an existing interaction with the same trigger on this node"),
            )
            .build(),
        handler(|ctx: Ctx, args: Value| async move {
            let action = js_string(get(&args, "action"));
            let needs_target = !["BACK", "CLOSE", "URL"].contains(&action.as_str());
            if needs_target && !truthy(get(&args, "destinationId")) {
                return Err(anyhow!("action {action} needs destinationId"));
            }
            if action == "URL" && !truthy(get(&args, "url")) {
                return Err(anyhow!("action URL needs url"));
            }
            Ok(text(&ctx.figma("add_interaction", args, None).await?))
        }),
    );

    d.fwd(
        "remove_interactions",
        "Remove all interactions from a node, or only those with the given trigger.",
        shape().f("nodeId", node_id()).f("trigger", string().optional()).build(),
    );

    d.fwd(
        "set_flow_start",
        "Mark a top-level frame as a prototype flow starting point (or remove it). The first flow is the default when presenting.",
        shape()
            .f("nodeId", node_id())
            .f("name", string().optional().describe("Flow name, e.g. Onboarding"))
            .f("remove", boolean().default(json!(false)))
            .build(),
    );

    d.fwd(
        "set_scroll",
        "Set a frame's scroll behavior in the prototype and which direct children stay fixed (header, tab bar). Fixed layers are moved on top; inside auto layout, make them absolute (set_props) first.",
        shape()
            .f("nodeId", node_id())
            .f("overflow", enumeration(&["NONE", "HORIZONTAL", "VERTICAL", "BOTH"]).optional())
            .f("fixedNodeIds", array(string()).optional().describe("Direct children to keep fixed; [] clears"))
            .build(),
    );

    // ---- design system ----

    d.fwd(
        "apply_style",
        "Apply local style ids (from get_styles) to a node.",
        shape()
            .f("nodeId", node_id())
            .f("fillStyleId", string().optional())
            .f("strokeStyleId", string().optional())
            .f("textStyleId", string().optional())
            .f("effectStyleId", string().optional())
            .build(),
    );

    d.fwd(
        "bind_variable",
        "Bind a variable (from get_variables) to a node field; fill/stroke bind the color.",
        shape().f("nodeId", node_id()).f("field", enumeration(BINDABLE_FIELDS)).f("variableId", string()).build(),
    );

    let variable_value = || {
        union(vec![
            string(),
            number(),
            boolean(),
            object(shape().f("alias", string().describe("Variable id or name")).build()),
        ])
    };
    let variable_type = || enumeration(&["COLOR", "FLOAT", "STRING", "BOOLEAN"]);

    d.fwd(
        "create_variable_collection",
        "Create a variable collection with modes, e.g. {name: \"Color\", modes: [\"Light\", \"Dark\"]} (first mode is the default). The Figma plan may limit the number of modes.",
        shape()
            .f("name", string().min(1.0))
            .f("modes", array(string().min(1.0)).min(1.0).max(10.0).default(json!(["Default"])))
            .build(),
    );

    d.fwd(
        "create_variables",
        "Create many variables in one call. values are per mode name: hex for COLOR, number for FLOAT, or {alias: \"Color/Primary/500\"} to reference another variable. Slash names group them (Color/Primary/500, Spacing/4).",
        shape()
            .f("collectionId", string())
            .f(
                "variables",
                array(object(
                    shape()
                        .f("name", string().min(1.0))
                        .f("type", variable_type())
                        .f("values", record(variable_value()))
                        .f("scopes", array(string()).optional().describe("Where it can be used, e.g. ALL_FILLS, TEXT_FILL, STROKE_COLOR, GAP, CORNER_RADIUS, FONT_SIZE"))
                        .build(),
                ))
                .min(1.0)
                .max(500.0),
            )
            .build(),
    );

    d.fwd(
        "set_variable_value",
        "Change a variable's value for one mode.",
        shape()
            .f("variableId", string())
            .f("mode", string().describe("Mode name or id"))
            .f("value", variable_value())
            .build(),
    );

    d.fwd(
        "set_instance_properties",
        "Set component properties on an instance (names as shown by get_node).",
        shape().f("nodeId", node_id()).f("properties", record(union(vec![string(), boolean()]))).build(),
    );

    d.0
}

// ---------- design → code helpers ----------

/// `[...].map(String)` as a JSON list.
fn strings(items: &[String]) -> Value {
    Value::Array(items.iter().map(|s| Value::String(s.clone())).collect())
}

/// codegen.mjs `readComponentMap(dir)`, with V8's `JSON.parse` message when the file is not JSON (the
/// crate reports serde's).
fn read_component_map(dir: &str) -> anyhow::Result<Value> {
    codegen::read_component_map(dir).map_err(|err| {
        let file = node_path::join(&[dir, COMPONENT_MAP_FILE]);
        match std::fs::read(&file).map(|b| crate::jsjson::parse(&String::from_utf8_lossy(&b))) {
            Ok(Err(message)) => anyhow!("Cannot read {file}: {message}"),
            _ => err,
        }
    })
}

/// codegen.mjs `kebab(value)` on whatever the plugin sent, throwing like JS when it is not a string.
fn kebab_of(v: Option<&Value>) -> anyhow::Result<String> {
    match v {
        Some(Value::String(s)) => Ok(codegen::kebab(s)),
        None | Some(Value::Null) => Err(crate::jsjson::read_error(v, "normalize")),
        Some(_) => Err(anyhow!("name.normalize is not a function")),
    }
}

/// export_assets' `write(asset, name, dir)`: `<dir>/<name>.<format>` from the asset's base64.
fn write_asset(asset: Option<&Value>, name: &str, dir: &str, files: &mut Vec<Value>) -> anyhow::Result<()> {
    let format = prop(asset, "format")?.cloned();
    let file = node_path::join(&[dir, &format!("{name}.{}", fjs::to_string(format.as_ref()))]);
    let bytes = crate::jsjson::buffer_from_base64(prop(asset, "base64")?)?;
    crate::util::write_file_deep(&file, &bytes)?;
    files.push(obj! { "nodeId" => prop(asset, "nodeId")?.cloned(), "file" => file.replace('\\', "/"), "format" => format, "bytes" => bytes.len() });
    Ok(())
}

// ---------- motion handoff helpers ----------

/// Which section a frame sits in (its name decides the motion context).
async fn section_of(ctx: &Ctx, id: Option<&Value>) -> anyhow::Result<Value> {
    let doc = ctx.figma("get_document_info", json!({}), None).await?;
    for top in arr(fjs::nullish(prop(Some(&doc), "topLevel")?)) {
        if prop(Some(&top), "type")? != Some(&json!("SECTION")) {
            continue;
        }
        let sec = ctx.figma("get_node", obj! { "nodeId" => top.get("id").cloned(), "depth" => 1i64 }, None).await?;
        for c in arr(fjs::nullish(prop(Some(&sec), "children")?)) {
            if fjs::strict_eq(prop(Some(&c), "id")?, id) {
                return Ok(top.get("name").cloned().unwrap_or(Value::Null));
            }
        }
    }
    Ok(Value::Null)
}

/// What `handoff()` collects: each frame's motion_read, the classified rows and the context used.
struct Handoff {
    reads: Vec<Value>,
    rows: Vec<Row>,
    warnings: Vec<String>,
    context: String,
}

/// Read one or more frames' Motion and classify every animated layer (code + tokens vs Lottie). The section a
/// frame sits in decides the context: a "… · Storyboard · …" / landing section is landing motion.
async fn handoff(ctx: &Ctx, ids: &[Value], context: &str) -> anyhow::Result<Handoff> {
    let context = if context == "auto" {
        let section = section_of(ctx, ids.first()).await.unwrap_or(Value::Null);
        motion_handoff::motion_context(Some(&section)).to_string()
    } else {
        context.to_string()
    };
    let mut h = Handoff {
        reads: vec![],
        rows: vec![],
        warnings: vec![],
        context,
    };
    for id in ids {
        let r = ctx.figma("motion_read", obj! { "nodeId" => id }, None).await?;
        let rows = motion_handoff::classify(&r, &mut h.warnings, &h.context)?;
        h.reads.push(r);
        h.rows.extend(rows);
    }
    Ok(h)
}

// ---------- Lottie ----------

/// What `writeLottie()` gets besides the tool arguments (projectDir, out, nodeId, svg, svgFile).
struct LottieJob<'a> {
    args: &'a Value,
    spec: Map<String, Value>,
    source: &'static str,
    extra_warnings: Vec<String>,
}

/// writeLottie(): compile and write a Lottie into the project; the recipe rides along in meta (players
/// ignore it): /motion shows it, an edit starts from it. A GIF already sent to Figma stays linked; /motion
/// marks it stale until it is sent again.
async fn write_lottie(ctx: &Ctx, job: LottieJob<'_>) -> anyhow::Result<Value> {
    let LottieJob {
        args,
        spec,
        source,
        extra_warnings,
    } = job;
    let out = js_string(get(args, "out"));
    let id = get(args, "nodeId").cloned();
    let svg_file = get(args, "svgFile").cloned();
    let root = node_path::resolve(&[&js_string(get(args, "projectDir"))]);
    let target = node_path::resolve(&[&root, &out]);
    let rel = node_path::relative(&root, &target);
    if rel.starts_with("..") || node_path::is_absolute(&rel) {
        return Err(anyhow!("out must be inside {root}"));
    }
    let mut art = get(args, "svg").cloned().unwrap_or(Value::Null);
    if !truthy(Some(&art)) && truthy(svg_file.as_ref()) {
        let f = js_string(svg_file.as_ref());
        let path = if node_path::is_absolute(&f) { f } else { node_path::join(&[&root, &f]) };
        art = Value::String(String::from_utf8_lossy(&crate::util::read_file(Path::new(&path))?).into_owned());
    }
    if !truthy(Some(&art)) && truthy(id.as_ref()) {
        let items = json!([obj! { "nodeId" => id.clone(), "format" => "svg", "scale" => 1i64 }]);
        let asset = first_of(ctx.figma("export_assets", json!({ "items": items }), None).await?)?;
        let bytes = crate::jsjson::buffer_from_base64(prop(asset.as_ref(), "base64")?)?;
        art = Value::String(String::from_utf8_lossy(&bytes).into_owned());
    }
    if !truthy(Some(&art)) {
        return Err(anyhow!("Give the art: nodeId, svg or svgFile"));
    }
    // compileLottie({ ...spec, name: spec.name ?? <file name without .json>, svg: art })
    let mut compile = spec.clone();
    if fjs::nullish(spec.get("name")).is_none() {
        let last = out.rsplit(['\\', '/']).next().unwrap_or("");
        let name = if last.to_ascii_lowercase().ends_with(".json") { &last[..last.len() - 5] } else { last };
        compile.insert("name".into(), Value::String(name.to_string()));
    }
    compile.insert("svg".into(), art);
    let compiled = lottie::compile_lottie(&Value::Object(compile))?;
    let mut lottie = compiled.get("lottie").cloned().unwrap_or(Value::Null);
    // the GIF component an earlier version was sent to (meta.wara.figma) stays linked
    let previous = std::fs::read(&target)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|j| j.get("meta").and_then(|m| m.get("wara")).and_then(|w| w.get("figma")).cloned());
    // spec: { ...spec, nodeId: id, svgFile } — undefined values are left out, as JSON.stringify does
    let mut recipe = spec;
    for (key, value) in [("nodeId", id), ("svgFile", svg_file)] {
        match value {
            Some(v) => {
                recipe.insert(key.into(), v);
            }
            None => {
                recipe.shift_remove(key);
            }
        }
    }
    let mut wara = obj! { "source" => source, "spec" => Value::Object(recipe) };
    if let Some(figma) = previous.filter(|f| truthy(Some(f))) {
        wara.as_object_mut().unwrap().insert("figma".into(), figma);
    }
    if let Value::Object(o) = &mut lottie {
        o.insert("meta".into(), obj! { "g" => "wara-bridge", "wara" => wara });
    }
    let text = fjs::stringify(&lottie);
    crate::util::write_file_deep(&target, text.as_bytes())?;
    let mut lines = vec![format!(
        "Wrote {} ({} KB){}",
        node_path::relative(&root, &target).replace('\\', "/"),
        crate::util::kilobytes(len_utf16(&text)),
        if source == "figma-motion" { " from the Figma Motion timeline" } else { "" }
    )];
    lines.push(fjs::to_string(compiled.get("summary")));
    let compiled_warnings = arr(compiled.get("warnings"));
    let warnings = extra_warnings.into_iter().chain(compiled_warnings.iter().map(|w| js_string(Some(w))));
    lines.extend(warnings.map(|w| format!("⚠ {w}")));
    lines.push("Play it with lottie-react / lottie-web (web) or the lottie package (Flutter); loop is a player option.".into());
    Ok(text_result(lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_map_text() {
        let map = json!({
            "file": "utils", "foundations": true,
            "pages": [{"id": "0:1", "name": "App", "flows": [{"name": "Main", "nodeId": "1:1"}], "screens": [
                {"id": "1:1", "name": "Home", "size": "375×812", "status": "approved", "components": [["Button", 2], ["Card", 1]], "to": ["Detail"]},
                {"id": "1:2", "name": "Detail", "size": "375×812", "components": [], "to": []}
            ]}],
            "components": [["Button", 2, 1]], "unusedComponents": ["Tag", "Chip"], "breakpoints": {"Home": {"base": "1:1", "md": "1:2"}},
            "otherWork": [{"name": "Pitch", "kind": "slides"}]
        });
        let got = render_design_map(&map, 1);
        assert_eq!(
            got,
            "File \"utils\" · 1 page(s) · 2 screens · 1 components in use · Foundations: yes\nPrototype work only (sections \"<Flow> · Prototype · Draft / In review / Approved\"; old \"<Flow> · <Status>\" and loose frames count too): 1 approved · 0 in review · 0 draft · 1 without a review section (not approved)\n\n## Page \"App\" (0:1) — 2 screens\nFlows: Main → 1:1\n- Home (1:1) 375×812 · APPROVED · uses Button×2, Card · → Detail\n… 1 more screens (pass pageId or maxScreens)\n\n## Breakpoint sets\n- Home: base 1:1, md 1:2\n\n## Components (instances / screens)\nButton 2/1\n\nUnused local components: Tag, Chip\n\n## Other work (not in the design graph)\n- Pitch [slides]"
        );
    }
}
