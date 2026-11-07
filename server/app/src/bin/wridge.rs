//! `wridge`: the command line client (what `node legacy-node/wara.mjs` was).

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(wridge_server::cli::main(args).await);
}
