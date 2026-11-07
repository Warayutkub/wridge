//! `cargo run --release --example update_graph -- <absolute project dir> [--force]`
//! Builds/updates `<dir>/.wara/code-graph.db` and prints the info object as JSON (like
//! `JSON.stringify(await updateGraph(dir, { force }))` in Node).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let force = args.iter().any(|a| a == "--force");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: update_graph <absolute project dir> [--force]");
        std::process::exit(2);
    };
    match wridge_graph::update_graph(dir, wridge_graph::UpdateOptions { force }) {
        Ok(info) => println!("{}", wridge_graph::js::json_stringify(&info)),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
