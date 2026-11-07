//! `cli <code-graph|memory> [args…]`: runs `wridge_graph::cli` like `node wara.mjs code-graph|memory …`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    let out = match args.first().map(String::as_str) {
        Some("memory") => wridge_graph::cli::memory_cli(&args[1..], &cwd, || std::io::read_to_string(std::io::stdin()).unwrap_or_default()),
        _ => wridge_graph::cli::code_graph_cli(&args[1.min(args.len())..], &cwd),
    };
    match out {
        Ok(t) if !t.is_empty() => println!("{t}"),
        Ok(_) => {}
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
