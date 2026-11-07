//! Next.js routes from the file system (port of `routes-next.mjs`): App Router (`app/`,
//! `src/app/`), Pages Router and proxy/middleware.

use crate::paths::dir_key;
use crate::re;

use std::path::Path;

const APP_FILES: [&str; 11] = [
    "page",
    "layout",
    "template",
    "loading",
    "error",
    "global-error",
    "not-found",
    "route",
    "default",
    "forbidden",
    "unauthorized",
];

fn code() -> &'static crate::jsre::JsRegex {
    re!(r"\.(tsx|ts|jsx|js|mjs)$")
}

/// A route row before ids are known.
#[derive(Clone, Debug, PartialEq)]
pub struct NextRoute {
    pub file: String,
    pub framework: String,
    pub path: String,
    pub kind: String,
    pub root: String,
}

/// Folders holding a package.json that depends on next.
pub fn next_roots(root: &Path, paths: &[String]) -> Vec<String> {
    let mut roots = Vec::new();
    for p in paths {
        if !p.ends_with("package.json") {
            continue;
        }
        let Ok(text) = crate::js::read_utf8(root.join(p)) else { continue };
        let Ok(json) = crate::js::json_parse(&text) else { continue }; // invalid package.json
        let dep = |k: &str| json.get(k).filter(|d| !d.is_null()).and_then(|d| d.get("next"));
        if crate::js::truthy(dep("dependencies")) || crate::js::truthy(dep("devDependencies")) {
            roots.push(dir_key(p));
        }
    }
    roots
}

fn join2(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join("/")
}

/// `nextRoutes(root, paths)`.
pub fn next_routes(root: &Path, paths: &[String]) -> Vec<NextRoute> {
    let mut routes = Vec::new();
    let code = code();
    for r in next_roots(root, paths) {
        for base in ["app", "src/app"].map(|d| join2(&[&r, d])) {
            let prefix = format!("{base}/");
            for p in paths {
                if !p.starts_with(&prefix) || !code.test_str(p) {
                    continue;
                }
                let rel = &p[base.len() + 1..];
                let mut segs: Vec<&str> = rel.split('/').collect();
                let kind = code.replace_first_str(segs.pop().unwrap_or(""), "");
                if !APP_FILES.contains(&kind.as_str()) {
                    continue;
                }
                if segs.iter().any(|s| s.starts_with('_')) {
                    continue; // private folder: not routable
                }
                let url: Vec<String> = segs
                    .iter()
                    .filter(|s| !re!(r"^\(.*\)$").test_str(s) && !s.starts_with('@')) // route groups and parallel slots don't appear in the URL
                    .map(|s| re!(r"^(\(\.{1,3}\))+").replace_first_str(s, "")) // intercepting routes: (.)photo, (..)(..)photo
                    .collect();
                routes.push(NextRoute {
                    file: p.clone(),
                    framework: "next-app".into(),
                    path: format!("/{}", url.join("/")),
                    kind,
                    root: base.clone(),
                });
            }
        }
        for base in ["pages", "src/pages"].map(|d| join2(&[&r, d])) {
            let prefix = format!("{base}/");
            for p in paths {
                if !p.starts_with(&prefix) || !code.test_str(p) {
                    continue;
                }
                let rel = code.replace_first_str(&p[base.len() + 1..], "");
                let name = rel.split('/').next_back().unwrap_or("").to_string();
                if ["_app", "_document", "_error"].contains(&name.as_str()) {
                    routes.push(NextRoute {
                        file: p.clone(),
                        framework: "next-pages".into(),
                        path: "*".into(),
                        kind: name[1..].to_string(),
                        root: base.clone(),
                    });
                    continue;
                }
                let url = format!("/{}", re!(r"(^|\/)index$").replace_first_str(&rel, ""));
                let kind = if rel.starts_with("api/") { "route" } else { "page" };
                routes.push(NextRoute {
                    file: p.clone(),
                    framework: "next-pages".into(),
                    path: url,
                    kind: kind.into(),
                    root: base.clone(),
                });
            }
        }
        for name in ["proxy", "middleware"] {
            for dir in [r.clone(), join2(&[&r, "src"])] {
                let want = join2(&[&dir, name]);
                if let Some(p) = paths.iter().find(|x| code.test_str(x) && code.replace_first_str(x, "") == want) {
                    let kind = if name == "proxy" { "proxy" } else { "middleware" };
                    routes.push(NextRoute {
                        file: p.clone(),
                        framework: "next-app".into(),
                        path: "*".into(),
                        kind: kind.into(),
                        root: if dir.is_empty() { ".".into() } else { dir.clone() },
                    });
                }
            }
        }
    }
    routes
}

/// `"/jobs/[id]"` matches `"/jobs/42"`; catch-alls match the rest.
pub fn route_matches(pattern: &str, url: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let u: Vec<&str> = url.split('?').next().unwrap_or("").split('/').filter(|s| !s.is_empty()).collect();
    for (i, seg) in p.iter().enumerate() {
        if re!(r"^\[\[\.\.\..+\]\]$").test_str(seg) {
            return true;
        }
        if re!(r"^\[\.\.\..+\]$").test_str(seg) {
            return u.len() > i;
        }
        if i >= u.len() {
            return false;
        }
        if !re!(r"^\[.+\]$").test_str(seg) && *seg != u[i] {
            return false;
        }
    }
    p.len() == u.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_and_pages_routes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"dependencies":{"next":"16"}}"#).unwrap();
        let paths: Vec<String> = [
            "package.json",
            "app/(shop)/jobs/[id]/page.tsx",
            "app/layout.tsx",
            "app/_lib/x.ts",
            "app/@modal/(.)photo/page.tsx",
            "pages/index.tsx",
            "pages/api/hello.ts",
            "pages/_app.tsx",
            "middleware.ts",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let got: Vec<(String, String)> = next_routes(dir.path(), &paths).into_iter().map(|r| (r.path, r.kind)).collect();
        let want: Vec<(String, String)> = [
            ("/jobs/[id]", "page"),
            ("/", "layout"),
            ("/photo", "page"),
            ("/", "page"),
            ("/api/hello", "route"),
            ("*", "app"),
            ("*", "middleware"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(got, want);
        assert!(route_matches("/jobs/[id]", "/jobs/42?x=1"));
        assert!(route_matches("/docs/[[...slug]]", "/docs"));
        assert!(!route_matches("/jobs/[id]", "/jobs"));
    }
}
