//! Port of legacy-node/test/pdf.test.mjs.
use lopdf::{dictionary, Document, Object};
use serde_json::{json, Value};
use std::cell::RefCell;
use wridge_figma::pdf::{design_pdf, merge_pdfs, order_pages, page_number, pdf_file_name};

fn frame(name: &str, x: f64, y: f64) -> Value {
    json!({ "id": name, "name": name, "type": "FRAME", "x": x, "y": y, "width": 595, "height": 842 })
}
fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}
fn pn(s: &str) -> Option<f64> {
    page_number(Some(&json!(s)))
}

#[test]
fn page_number_leading_digits_as_a_number() {
    assert_eq!(pn("01 Cover"), Some(1.0));
    assert_eq!(pn("100 ภาคผนวก"), Some(100.0));
    assert_eq!(pn("  7"), Some(7.0));
    assert_eq!(pn("Cover"), None);
    assert_eq!(pn("A1"), None);
}

#[test]
fn pages_sort_as_numbers_not_as_text() {
    let o = order_pages(Some(&json!([frame("1000 z", 0.0, 0.0), frame("10 c", 0.0, 0.0), frame("2 b", 900.0, 0.0), frame("100 d", 0.0, 0.0), frame("01 a", 2000.0, 0.0)])));
    assert_eq!(names(&o["pages"]), ["01 a", "2 b", "10 c", "100 d", "1000 z"]);
    assert_eq!(o["warnings"], json!([]));
}

#[test]
fn unnumbered_frames_go_last_by_position_hidden_and_non_frames_are_left_out() {
    let mut hidden = frame("3 hidden", 0.0, 0.0);
    hidden["visible"] = json!(false);
    let o = order_pages(Some(&json!([
        frame("note", 0.0, 900.0), frame("2 b", 0.0, 0.0), frame("cover", 0.0, 0.0), frame("1 a", 0.0, 0.0),
        hidden, { "id": "t", "name": "4 text", "type": "TEXT", "x": 0, "y": 0 },
    ])));
    assert_eq!(names(&o["pages"]), ["1 a", "2 b", "cover", "note"]);
    assert_eq!(o["skipped"], json!(["3 hidden"]));
    assert_eq!(o["warnings"].as_array().unwrap().len(), 2);
}

#[test]
fn duplicate_numbers_keep_position_order_and_warn() {
    let o = order_pages(Some(&json!([frame("2 right", 700.0, 0.0), frame("2 left", 0.0, 0.0), frame("1", 0.0, 0.0)])));
    assert_eq!(names(&o["pages"]), ["1", "2 left", "2 right"]);
    assert!(o["warnings"][0].as_str().unwrap().contains("number 2 is used twice"));
}

fn one_page(w: i64, h: i64) -> Vec<u8> {
    let mut d = Document::with_version("1.7");
    let pages = d.new_object_id();
    let page = d.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()], "Resources" => dictionary! {} });
    d.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
    let catalog = d.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    d.trailer.set("Root", catalog);
    let mut out = vec![];
    d.save_to(&mut out).unwrap();
    out
}

fn sizes(pdf: &[u8]) -> Vec<(i64, i64)> {
    let d = Document::load_mem(pdf).unwrap();
    d.get_pages()
        .values()
        .map(|id| {
            let mb = d.get_dictionary(*id).unwrap().get(b"MediaBox").unwrap().as_array().unwrap().clone();
            let f = |o: &Object| o.as_float().map(|x| x as f64).or_else(|_| o.as_i64().map(|x| x as f64)).unwrap();
            ((f(&mb[2]) - f(&mb[0])).round() as i64, (f(&mb[3]) - f(&mb[1])).round() as i64)
        })
        .collect()
}

fn title(pdf: &[u8]) -> String {
    let d = Document::load_mem(pdf).unwrap();
    let info = d.trailer.get(b"Info").unwrap().as_reference().unwrap();
    let Object::String(bytes, _) = d.get_dictionary(info).unwrap().get(b"Title").unwrap() else { panic!() };
    let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).unwrap()
}

#[test]
fn merge_keeps_every_page_in_order() {
    let merged = merge_pdfs(&[one_page(595, 842), one_page(842, 595), one_page(100, 200)], Some("Book")).unwrap();
    assert_eq!(sizes(&merged), [(595, 842), (842, 595), (100, 200)]);
    assert_eq!(title(&merged), "Book");
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

#[test]
fn design_pdf_section_to_ordered_export_title_without_kind_status() {
    let pdf = one_page(595, 842);
    let asked = RefCell::new(vec![]);
    let send = |cmd: String, args: Value, _timeout: Option<u64>| {
        let r = if cmd == "get_node" {
            json!({ "id": "s", "name": "Laravel DOC 1 · Doc · Draft", "type": "SECTION", "children": [frame("2 b", 0.0, 0.0), frame("1 a", 0.0, 0.0)] })
        } else {
            asked.borrow_mut().push(args["items"][0]["nodeId"].as_str().unwrap().to_string());
            assert_eq!(args["items"][0]["format"], "pdf");
            json!([{ "base64": b64(&pdf) }])
        };
        std::future::ready(Ok(r))
    };
    let out = block_on(design_pdf(send, &json!("s"))).unwrap();
    assert_eq!(asked.into_inner(), ["1 a", "2 b"]);
    assert_eq!(out.title.as_deref(), Some("Laravel DOC 1"));
    assert_eq!(Document::load_mem(&out.pdf).unwrap().get_pages().len(), 2);
    assert_eq!(pdf_file_name(Some(&json!("a/b: \"c\""))), "a-b- -c-.pdf");
}

#[test]
fn design_pdf_refuses_nodes_that_are_not_a_section_or_frame() {
    let e = block_on(design_pdf(|_, _, _| std::future::ready(Ok(json!({ "name": "x", "type": "TEXT" }))), &json!("1"))).unwrap_err();
    assert!(e.to_string().contains("pick a section"));
    let e = block_on(design_pdf(|_, _, _| std::future::ready(Ok(json!({ "name": "empty", "type": "SECTION", "children": [] }))), &json!("1"))).unwrap_err();
    assert!(e.to_string().contains("No frames"));
}
