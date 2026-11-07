//! Port of `legacy-node/pdf.mjs`: Figma frames → one PDF, a page per frame, like a document or a book.
//! Order: the number at the start of the frame name ("1 Cover", "02 Intro", "100 …"), compared as numbers so
//! no zero-padding is needed; frames without a number follow, top-to-bottom then left-to-right.
//! Merging uses lopdf instead of pdf-lib: the same pages, page sizes and title, not the same bytes.

use crate::js::{self, num};
use crate::obj;
use anyhow::{anyhow, Result};
use base64::Engine;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, StringFormat};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::sync::OnceLock;

const PAGE_TYPES: [&str; 3] = ["FRAME", "COMPONENT", "INSTANCE"];

fn is_page_type(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::String(t)) if PAGE_TYPES.contains(&t.as_str()))
}

/// `pageNumber(name)`: the leading digits as a number, or None.
pub fn page_number(name: Option<&Value>) -> Option<f64> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(&format!("^[{}]*([0-9]+)", js::WS)).unwrap());
    let s = js::to_string(name);
    re.captures(&s).map(|m| js::str_to_number(&m[1]))
}

/// `orderPages(children)`: get_node children of a section (or page) → `{ pages, skipped, warnings, issues }`;
/// issues = `{ duplicates: [{ n, names }], unnumbered: [name], hidden: [name] }`, the warnings for a UI to draw.
pub fn order_pages(children: Option<&Value>) -> Value {
    let list: Vec<Value> = match js::nullish(children) {
        Some(Value::Array(a)) => a.clone(),
        _ => vec![],
    };
    let get = |c: &Value, k: &str| js::get(Some(c), k).cloned();
    let num_of = |c: &Value, k: &str| js::to_number(js::get(Some(c), k));
    let by_place = |a: &Value, b: &Value| {
        let d = num_of(a, "y") - num_of(b, "y");
        if d != 0.0 && !d.is_nan() {
            d
        } else {
            num_of(a, "x") - num_of(b, "x")
        }
    };
    let candidates: Vec<&Value> = list.iter().filter(|c| is_page_type(js::get(Some(c), "type"))).collect();
    let hidden = |c: &Value| js::get(Some(c), "visible") == Some(&Value::Bool(false));
    let skipped: Vec<Value> = candidates.iter().filter(|c| hidden(c)).map(|c| get(c, "name").unwrap_or(Value::Null)).collect();
    let shown: Vec<(&Value, Option<f64>)> = candidates.iter().filter(|c| !hidden(c)).map(|c| (*c, page_number(js::get(Some(c), "name")))).collect();
    let mut numbered: Vec<(&Value, f64)> = shown.iter().filter_map(|(c, n)| n.map(|n| (*c, n))).collect();
    numbered.sort_by(|a, b| {
        let d = a.1 - b.1;
        js::cmp_num(if d != 0.0 && !d.is_nan() { d } else { by_place(a.0, b.0) })
    });
    let mut loose: Vec<&Value> = shown.iter().filter(|(_, n)| n.is_none()).map(|(c, _)| *c).collect();
    loose.sort_by(|a, b| js::cmp_num(by_place(a, b)));
    let mut seen: Vec<(f64, Vec<Value>)> = vec![];
    for (c, n) in &numbered {
        let name = get(c, "name").unwrap_or(Value::Null);
        match seen.iter_mut().find(|(k, _)| k == n) {
            Some(e) => e.1.push(name),
            None => seen.push((*n, vec![name])),
        }
    }
    let duplicates: Vec<&(f64, Vec<Value>)> = seen.iter().filter(|(_, names)| names.len() > 1).collect();
    let unnumbered: Vec<Value> = if numbered.is_empty() { vec![] } else { loose.iter().map(|c| get(c, "name").unwrap_or(Value::Null)).collect() };
    let mut warnings: Vec<String> = vec![];
    for (n, names) in &duplicates {
        warnings.push(format!("Page number {} is used twice: {} (ordered by position)", js::num_str(*n), js::join(names, ", ")));
    }
    if !unnumbered.is_empty() {
        warnings.push(format!("No number at the start of the name, so they go last: {}", js::join(&unnumbered, ", ")));
    }
    if !skipped.is_empty() {
        warnings.push(format!("Hidden, left out: {}", js::join(&skipped, ", ")));
    }
    let pages: Vec<Value> = numbered
        .iter()
        .map(|(c, _)| *c)
        .chain(loose.iter().copied())
        .map(|c| obj! { "id" => get(c, "id"), "name" => get(c, "name"), "width" => get(c, "width"), "height" => get(c, "height") })
        .collect();
    obj! {
        "pages" => pages,
        "skipped" => skipped.clone(),
        "warnings" => Value::Array(warnings.into_iter().map(Value::String).collect()),
        "issues" => obj! {
            "duplicates" => Value::Array(duplicates.iter().map(|(n, names)| obj! { "n" => num(*n), "names" => names.clone() }).collect()),
            "unnumbered" => unnumbered,
            "hidden" => skipped,
        },
    }
}

// The value of a page attribute, looked up the Pages tree like a reader does (MediaBox, Resources… inherit).
fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut current = page;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let node = doc.get_dictionary(current).ok()?;
        if let Ok(v) = node.get(key) {
            if !matches!(doc.dereference(v), Ok((_, Object::Null)) | Err(_)) {
                return Some(v.clone());
            }
        }
        current = node.get(b"Parent").ok()?.as_reference().ok()?;
    }
}

/// "D:YYYYMMDDHHmmSSZ" for now (pdf-lib's date format).
fn pdf_date() -> String {
    let iso = js::iso_now(); // 2026-11-06T12:34:56.789Z
    format!("D:{}{}{}{}{}{}Z", &iso[0..4], &iso[5..7], &iso[8..10], &iso[11..13], &iso[14..16], &iso[17..19])
}

/// A PDF text string as pdf-lib writes it (`PDFHexString.fromText`): UTF-16BE with a BOM, hex.
fn text_string(s: &str) -> Object {
    let mut bytes = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        bytes.extend(u.to_be_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

/// `mergePdfs(buffers, title)`: one-page (or more) PDFs from Figma, in order → one PDF.
pub fn merge_pdfs(buffers: &[Vec<u8>], title: Option<&str>) -> Result<Vec<u8>> {
    let mut out = Document::with_version("1.7");
    let pages_id = out.new_object_id();
    let mut next_id = out.max_id + 1;
    let mut kids: Vec<Object> = vec![];
    for buf in buffers {
        let mut src = Document::load_mem(buf).map_err(|e| anyhow!("Failed to parse PDF document: {e}"))?;
        if src.trailer.get(b"Encrypt").is_ok() || src.is_encrypted() {
            return Err(anyhow!("Input document to `PDFDocument.load` is encrypted. You can use `PDFDocument.load(..., {{ ignoreEncryption: true }})` if you wish to load the document anyway."));
        }
        // each page carries what it inherited from its Pages node, then loses the old tree
        let ids: Vec<ObjectId> = src.get_pages().into_values().collect();
        for id in &ids {
            let mut page = src.get_dictionary(*id)?.clone();
            for key in [&b"MediaBox"[..], b"CropBox", b"Resources", b"Rotate"] {
                if page.get(key).is_err() {
                    if let Some(v) = inherited(&src, *id, key) {
                        page.set(key.to_vec(), v);
                    }
                }
            }
            src.objects.insert(*id, Object::Dictionary(page));
        }
        src.renumber_objects_with(next_id);
        next_id = src.max_id + 1;
        let ids: Vec<ObjectId> = src.get_pages().into_values().collect();
        for (id, o) in std::mem::take(&mut src.objects) {
            out.objects.insert(id, o);
        }
        for id in ids {
            if let Ok(Object::Dictionary(d)) = out.get_object_mut(id) {
                d.set("Parent", Object::Reference(pages_id));
            }
            kids.push(Object::Reference(id));
        }
    }
    out.max_id = next_id - 1;
    if kids.is_empty() {
        // pdf-lib saves an empty document with one blank A4 page
        let page = out.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Resources" => Dictionary::new(),
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(595.28), Object::Real(841.89)],
        });
        kids.push(Object::Reference(page));
    }
    let count = kids.len() as i64;
    out.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = out.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    let mut info = Dictionary::new();
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        info.set("Title", text_string(t));
    }
    info.set("Producer", text_string("wridge"));
    let now = pdf_date();
    info.set("CreationDate", Object::String(now.clone().into_bytes(), StringFormat::Literal));
    info.set("ModDate", Object::String(now.into_bytes(), StringFormat::Literal));
    let info = out.add_object(Object::Dictionary(info));
    out.trailer.set("Root", catalog);
    out.trailer.set("Info", info);
    out.prune_objects();
    let mut bytes = vec![];
    out.save_to(&mut bytes)?;
    Ok(bytes)
}

/// `pdfFileName(name)`: a safe file name ending in .pdf.
pub fn pdf_file_name(name: Option<&Value>) -> String {
    static R: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (bad, ws) = R.get_or_init(|| (Regex::new(r#"[\\/:*?"<>|]+"#).unwrap(), Regex::new(&format!("[{}]+", js::WS)).unwrap()));
    let s = js::to_string(name);
    let s = bad.replace_all(&s, "-");
    let s = ws.replace_all(&s, " ");
    let s = js::trim(&s);
    format!("{}.pdf", if s.is_empty() { "design" } else { s })
}

/// What [`design_pdf`] returns: the merged PDF and what went into it.
#[derive(Debug, Clone)]
pub struct DesignPdf {
    pub pdf: Vec<u8>,
    pub title: Option<String>,
    pub pages: Vec<Value>,
    pub warnings: Vec<String>,
}

/// Base64 the way `Buffer.from(s, "base64")` reads it (url-safe letters, no padding, stray characters ignored).
pub fn decode_base64(s: &str) -> Vec<u8> {
    let clean: String = s
        .chars()
        .filter_map(|c| match c {
            '-' => Some('+'),
            '_' => Some('/'),
            c if c.is_ascii_alphanumeric() || c == '+' || c == '/' => Some(c),
            _ => None,
        })
        .collect();
    let clean = if clean.len() % 4 == 1 { &clean[..clean.len() - 1] } else { &clean[..] };
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent).with_decode_allow_trailing_bits(true),
    );
    engine.decode(clean).unwrap_or_default()
}

/// `designPdf(send, ids)`: sections (their frames as pages, in order) or single frames, one after another.
/// `ids`: one node id or a list; the document follows the list's order. `send(command, args, timeout_ms)` is
/// the bridge's command sender (the plugin round trip): `get_node` and `export_assets`.
pub async fn design_pdf<F, Fut>(mut send: F, ids: &Value) -> Result<DesignPdf>
where
    F: FnMut(String, Value, Option<u64>) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let list: Vec<Value> = match ids {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let mut pages: Vec<Value> = vec![];
    let mut warnings: Vec<String> = vec![];
    let mut titles: Vec<String> = vec![];
    for node_id in list {
        let node = send("get_node".into(), obj! { "nodeId" => node_id, "depth" => 1i64 }, None).await?;
        let n = Some(&node);
        let ty = js::get(n, "type");
        let (order_pages_list, order_warnings): (Vec<Value>, Vec<String>) = if ty == Some(&Value::String("SECTION".into())) {
            let o = order_pages(js::get(n, "children"));
            (
                o["pages"].as_array().cloned().unwrap_or_default(),
                o["warnings"].as_array().map(|w| w.iter().map(js::string).collect()).unwrap_or_default(),
            )
        } else if is_page_type(ty) {
            (vec![node.clone()], vec![])
        } else {
            return Err(anyhow!(
                "{} is a {}: pick a section (its frames become the pages) or a frame",
                js::to_string(js::get(n, "name")),
                js::to_string(ty)
            ));
        };
        if order_pages_list.is_empty() {
            return Err(anyhow!("No frames in {} to use as pages", js::to_string(js::get(n, "name"))));
        }
        pages.extend(order_pages_list);
        warnings.extend(order_warnings);
        titles.push(js::trim(js::to_string(js::get(n, "name")).split(" · ").next().unwrap_or("")).to_string());
    }
    let mut buffers = vec![];
    for page in &pages {
        let args = obj! { "items" => vec![obj! { "nodeId" => js::get(Some(page), "id").cloned(), "format" => "pdf", "scale" => 1i64 }] };
        let res = send("export_assets".into(), args, Some(120_000)).await?;
        let b64 = js::get(res.as_array().and_then(|a| a.first()), "base64").map(js::string).unwrap_or_default();
        buffers.push(decode_base64(&b64));
    }
    let title = if titles.len() > 1 { Some(format!("{} +{}", titles[0], titles.len() - 1)) } else { titles.first().cloned() };
    let pdf = merge_pdfs(&buffers, title.as_deref())?;
    Ok(DesignPdf { pdf, title, pages, warnings })
}
