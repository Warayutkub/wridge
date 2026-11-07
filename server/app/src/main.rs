//! `wridge-server`: the wridge server (what `node legacy-node/index.mjs` was).

#[tokio::main]
async fn main() {
    wridge_server::server::run().await;
}
