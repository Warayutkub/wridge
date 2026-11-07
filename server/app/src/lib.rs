//! wridge server: the Rust port of legacy-node/index.mjs and its helpers. One module per JS file:
//! - `server` + `http` + `bridge` + `mcp` + `app`: index.mjs (lifecycle, HTTP API and pages, the plugin
//!   WebSocket, MCP over Streamable HTTP, the tool registry plumbing)
//! - `tools`: tools.mjs · `guards`: guards.mjs · `graph`: graph.mjs · `skills`: skills.mjs · `cli`: wara.mjs
//! - `zod` and `jsjson`: the parts of zod and of JavaScript's JSON/number handling the port must match
//! - `codegraph`: the projects list and the JS argument handling around wridge-graph (legacy-node/codegraph/**:
//!   code graph, brain, memory, knowledge, flows); the design-side modules (codegen, lottie, motion*, figjam,
//!   pdf) come from the wridge-figma crate

pub mod app;
pub mod bridge;
pub mod cli;
pub mod codegraph;
pub mod github;
pub mod graph;
pub mod guards;
pub mod http;
pub mod jsjson;
pub mod mcp;
pub mod paths;
pub mod server;
pub mod session;
pub mod skills;
pub mod tools;
pub mod util;
pub mod v8json;
pub mod zod;
