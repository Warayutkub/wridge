//! Times elkjs inside QuickJS: `cargo run --release --example elk_time [graph.json text]`.
use std::time::Instant;
fn main() {
    let g = std::env::args().nth(1).unwrap_or_else(|| r#"{"id":"root","layoutOptions":{"elk.algorithm":"layered","elk.direction":"RIGHT"},"children":[{"id":"a","width":100,"height":50},{"id":"b","width":100,"height":50},{"id":"c","width":80,"height":40}],"edges":[{"id":"e1","sources":["a"],"targets":["b"],"labels":[{"text":"x","width":28,"height":22}]},{"id":"e2","sources":["b"],"targets":["c"],"labels":[]},{"id":"e3","sources":["c"],"targets":["a"],"labels":[]}]}"#.to_string());
    let t = Instant::now();
    wridge_figma::elk::warm_up().unwrap();
    println!("warm-up {:?}", t.elapsed());
    for _ in 0..3 {
        let t = Instant::now();
        let r = wridge_figma::elk::layout(&g).unwrap();
        println!("layout {:?} {:?}", t.elapsed(), r);
    }
}
