//! wridge-figma: the design-side modules of the wridge server, ported from `legacy-node/`.
//!
//! | module | ports |
//! |---|---|
//! | [`codegen`] | `codegen.mjs` — token files (CSS / Tailwind / Dart), component map, responsive merge |
//! | [`lottie`] | `lottie.mjs` — SVG + motion spec → Lottie JSON |
//! | [`motion`] | `motion.mjs` — Lottie files in a project (list, read, mark sent to Figma) |
//! | [`motion_figma`] | `motion-figma.mjs` — Figma Motion ↔ spec |
//! | [`motion_handoff`] | `motion-handoff.mjs` — classify, lint, tokens, CSS / Framer Motion |
//! | [`figjam`] | `figjam.mjs` — diagram layout with the unchanged elkjs ([`elk`], QuickJS) |
//! | [`pdf`] | `pdf.mjs` — page order, merge (lopdf), design → PDF |
//!
//! Values are `serde_json::Value` shaped like the JS objects (camelCase keys, same order, integers as
//! integers). [`js`] holds the JavaScript semantics they rely on; write results with [`js::stringify`]
//! to get the bytes `JSON.stringify` would. [`node_path`] is Node's `path` module.
//!
//! Parity with the Node originals: `node server/figma/parity/parity.mjs` (from the repo root).
pub mod codegen;
pub mod elk;
pub mod figjam;
pub mod js;
pub mod lottie;
pub mod motion;
pub mod motion_figma;
pub mod motion_handoff;
pub mod node_path;
pub mod pdf;
