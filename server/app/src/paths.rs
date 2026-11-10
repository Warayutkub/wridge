//! Where the server finds its files. Node used paths relative to legacy-node/ (`HERE`):
//!   web/ (pages), AI_GUIDE.md, skills/  → the wridge folder (`root`)
//!   vendor files, folder-picker.ps1, wara.log → this crate's folder (`app_dir`), so no node_modules is needed.
//!   legacy-node/code-projects.json (the projects list, shared with the Node server) → `projects_file`
//! `WRIDGE_ROOT` / `WRIDGE_APP_DIR` override both (a packaged app points them at its resources);
//! `WRIDGE_PROJECTS_FILE` points the projects list elsewhere (tests use a temp copy).
//! `WRIDGE_DATA_DIR` is the person's own folder that survives updates (the packaged app's data folder);
//! without it, `app_dir`.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub web: PathBuf,
    pub guide: PathBuf,
    pub skills: PathBuf,
    pub app_dir: PathBuf,
    /// Kept across updates: tools the person downloaded (`tools/k6`).
    pub data_dir: PathBuf,
    pub vendor: PathBuf,
    pub folder_picker: PathBuf,
    pub log_file: PathBuf,
    /// The `wridge` CLI binary (wara.mjs's port), shown on the dashboard as cliPath.
    pub cli: PathBuf,
    /// The code-graph projects list (`code-projects.json`): `WRIDGE_PROJECTS_FILE`, else the Node server's
    /// own `legacy-node/code-projects.json`, so the user's list carries over.
    pub projects_file: PathBuf,
}

fn looks_like_root(dir: &Path) -> bool {
    dir.join("web").join("dashboard.html").is_file() && dir.join("skills").is_dir()
}

fn find_root() -> PathBuf {
    if let Ok(r) = std::env::var("WRIDGE_ROOT") {
        if !r.is_empty() {
            return PathBuf::from(r);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if looks_like_root(&d) {
                return d;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    // built inside wridge/server/app
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or(manifest)
}

impl Paths {
    pub fn discover() -> Paths {
        let root = find_root();
        let app_dir = match std::env::var("WRIDGE_APP_DIR") {
            Ok(d) if !d.is_empty() => PathBuf::from(d),
            _ => {
                let in_root = root.join("server").join("app");
                if in_root.join("vendor").is_dir() {
                    in_root
                } else {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                }
            }
        };
        Paths::new(root, app_dir)
    }

    pub fn new(root: PathBuf, app_dir: PathBuf) -> Paths {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| app_dir.clone());
        let cli = exe_dir.join(if cfg!(windows) { "wridge.exe" } else { "wridge" });
        let projects_file = match std::env::var("WRIDGE_PROJECTS_FILE") {
            Ok(f) if !f.is_empty() => PathBuf::from(f),
            _ => root.join("legacy-node").join("code-projects.json"),
        };
        let data_dir = match std::env::var("WRIDGE_DATA_DIR") {
            Ok(d) if !d.is_empty() => PathBuf::from(d),
            _ => app_dir.clone(),
        };
        Paths {
            projects_file,
            data_dir,
            web: root.join("web"),
            guide: root.join("AI_GUIDE.md"),
            skills: root.join("skills"),
            vendor: app_dir.join("vendor"),
            folder_picker: app_dir.join("folder-picker.ps1"),
            log_file: app_dir.join("wara.log"),
            cli,
            root,
            app_dir,
        }
    }

    /// The Dart parser folder (`legacy-node/codegraph/dart`, built there by the Node server) for
    /// wridge-graph, unless `WRIDGE_DART_DIR` names one.
    pub fn use_dart_dir(&self) {
        if std::env::var_os("WRIDGE_DART_DIR").is_some_and(|d| !d.is_empty()) {
            return;
        }
        let dir = self.root.join("legacy-node").join("codegraph").join("dart");
        if dir.is_dir() {
            wridge_graph::parse_dart::set_dart_dir(dir);
        }
    }
}
