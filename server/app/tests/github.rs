//! github.rs against a fake GitHub (axum on a random port): device flow, ETag/304, refresh, notifications.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use wridge_server::github::{Config, GitHub, MemoryStore, Saved, Store};

#[derive(Default)]
struct Fake {
    token_polls: usize,
    /// what the runs list holds: (id, status, conclusion, actor)
    runs: Vec<(u64, &'static str, &'static str, &'static str)>,
    runs_etag: usize,
    hits_200: usize,
    hits_304: usize,
    refreshes: usize,
    /// the access token the API accepts
    valid: String,
}
type S = Arc<Mutex<Fake>>;

fn form(body: &str) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}

async fn device_code(body: String) -> axum::Json<Value> {
    assert_eq!(form(&body)["client_id"], "test-client");
    axum::Json(json!({"device_code": "dev-1", "user_code": "WDGE-1234", "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 1}))
}

async fn access_token(State(s): State<S>, body: String) -> axum::Json<Value> {
    let f = form(&body);
    let mut st = s.lock().unwrap();
    if f["grant_type"] == "refresh_token" {
        assert_eq!(f["refresh_token"], "ref-1");
        st.refreshes += 1;
        st.valid = "tok-2".into();
        return axum::Json(json!({"access_token": "tok-2", "expires_in": 28800, "refresh_token": "ref-2", "refresh_token_expires_in": 15_000_000}));
    }
    assert_eq!(f["device_code"], "dev-1");
    st.token_polls += 1;
    if st.token_polls < 2 {
        return axum::Json(json!({"error": "authorization_pending"}));
    }
    st.valid = "tok-1".into();
    axum::Json(json!({"access_token": "tok-1", "expires_in": 28800, "refresh_token": "ref-1", "refresh_token_expires_in": 15_000_000}))
}

fn authed(s: &S, h: &HeaderMap) -> bool {
    let want = format!("Bearer {}", s.lock().unwrap().valid);
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(want.as_str())
}

async fn user(State(s): State<S>, h: HeaderMap) -> Response {
    if !authed(&s, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({"login": "octo"})).into_response()
}

async fn installations(State(s): State<S>, h: HeaderMap) -> Response {
    if !authed(&s, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({"installations": [{"id": 7}]})).into_response()
}

async fn inst_repos(Path(id): Path<u64>) -> axum::Json<Value> {
    assert_eq!(id, 7);
    axum::Json(json!({"repositories": [{"full_name": "octo/app", "name": "app", "owner": {"login": "octo"}, "private": true, "default_branch": "main", "html_url": "https://github.com/octo/app"}]}))
}

async fn runs(State(s): State<S>, h: HeaderMap) -> Response {
    if !authed(&s, &h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut st = s.lock().unwrap();
    let etag = format!("\"e{}\"", st.runs_etag);
    if h.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        st.hits_304 += 1;
        return (StatusCode::NOT_MODIFIED, [("etag", etag)]).into_response();
    }
    st.hits_200 += 1;
    let list: Vec<Value> = st.runs.iter().map(|(id, status, conclusion, actor)| json!({
        "id": id, "run_number": id, "name": "CI", "display_title": "Fix the build", "status": status,
        "conclusion": if conclusion.is_empty() { Value::Null } else { json!(conclusion) },
        "head_branch": "main", "head_sha": "abcdef123456", "event": "push", "actor": {"login": actor},
        "triggering_actor": {"login": actor}, "html_url": format!("https://github.com/octo/app/actions/runs/{id}"),
    })).collect();
    ([("etag", etag), ("x-ratelimit-remaining", "4999".to_string())], axum::Json(json!({"total_count": list.len(), "workflow_runs": list}))).into_response()
}

async fn jobs(Path(id): Path<u64>) -> axum::Json<Value> {
    axum::Json(json!({"jobs": [{"id": id * 10, "name": "build", "status": "completed", "conclusion": "failure",
        "steps": [{"number": 1, "name": "Set up job", "status": "completed", "conclusion": "success"}, {"number": 2, "name": "Test", "status": "completed", "conclusion": "failure"}]}]}))
}

async fn one_run(Path(id): Path<u64>) -> axum::Json<Value> {
    axum::Json(json!({"id": id, "run_number": id, "name": "CI", "status": "completed", "conclusion": "failure", "actor": {"login": "octo"}}))
}

async fn job_log() -> String {
    "2026-11-07T08:00:00.0000000Z npm test\n2026-11-07T08:00:01.0000000Z ##[error]Process completed with exit code 1.\n".into()
}

async fn start_fake() -> (S, String) {
    let s: S = Arc::new(Mutex::new(Fake::default()));
    let app = Router::new()
        .route("/login/device/code", post(device_code))
        .route("/login/oauth/access_token", post(access_token))
        .route("/user", get(user))
        .route("/user/installations", get(installations))
        .route("/user/installations/{id}/repositories", get(inst_repos))
        .route("/repos/octo/app/actions/runs", get(runs))
        .route("/repos/octo/app/actions/runs/{id}", get(one_run))
        .route("/repos/octo/app/actions/runs/{id}/jobs", get(jobs))
        .route("/repos/octo/app/actions/jobs/{id}/logs", get(job_log))
        .with_state(s.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (s, base)
}

fn github(base: &str, store: Box<dyn Store>) -> (Arc<GitHub>, Arc<Mutex<Vec<Value>>>) {
    let sent = Arc::new(Mutex::new(vec![]));
    let sink = sent.clone();
    let gh = GitHub::new(
        Config { client_id: "test-client".into(), api: base.into(), web: base.into() },
        store,
        Box::new(move |n| sink.lock().unwrap().push(n.clone())),
    );
    (gh, sent)
}

#[tokio::test]
async fn device_flow_signs_in_and_lists_repos() {
    let (fake, base) = start_fake().await;
    let (gh, _) = github(&base, Box::new(MemoryStore::default()));
    assert_eq!(gh.status()["state"], "signed_out");
    let s = gh.sign_in().await.unwrap();
    assert_eq!(s["state"], "signing_in");
    assert_eq!(s["signin"]["userCode"], "WDGE-1234");
    for _ in 0..50 {
        if gh.status()["state"] == "ready" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let s = gh.status();
    assert_eq!(s["state"], "ready", "{s}");
    assert_eq!(s["login"], "octo");
    assert_eq!(fake.lock().unwrap().token_polls, 2, "pending once, then the token");
    let r = gh.repos().await.unwrap();
    assert_eq!(r["repos"][0]["fullName"], "octo/app");
    // sign out forgets everything
    gh.sign_out();
    assert_eq!(gh.status()["state"], "signed_out");
}

#[tokio::test]
async fn cancel_stops_the_sign_in() {
    let (fake, base) = start_fake().await;
    let (gh, _) = github(&base, Box::new(MemoryStore::default()));
    gh.sign_in().await.unwrap();
    gh.cancel_sign_in();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(gh.status()["state"], "signed_out");
    assert_eq!(fake.lock().unwrap().token_polls, 0, "a cancelled sign-in stops polling");
}

fn signed_in(token: &str, expires_at: i64) -> Box<MemoryStore> {
    let store = MemoryStore::default();
    store
        .save(&Saved { access_token: token.into(), expires_at, refresh_token: "ref-1".into(), login: "octo".into(), notify_failed: true, ..Default::default() })
        .unwrap();
    Box::new(store)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[tokio::test]
async fn etag_makes_the_second_read_free() {
    let (fake, base) = start_fake().await;
    fake.lock().unwrap().valid = "tok-1".into();
    fake.lock().unwrap().runs = vec![(1, "completed", "success", "octo")];
    let (gh, _) = github(&base, signed_in("tok-1", 0));
    let a = gh.runs("octo/app").await.unwrap();
    let b = gh.runs("octo/app").await.unwrap();
    assert_eq!(a, b);
    assert_eq!(a["runs"][0]["sha"], "abcdef1");
    let f = fake.lock().unwrap();
    assert_eq!((f.hits_200, f.hits_304), (1, 1));
    assert_eq!(gh.status()["rateRemaining"], 4999);
    assert_eq!(gh.status()["watch"][0], "octo/app", "a repo you look at gets watched");
}

#[tokio::test]
async fn a_token_about_to_end_is_refreshed() {
    let (fake, base) = start_fake().await;
    fake.lock().unwrap().valid = "tok-2".into();
    let (gh, _) = github(&base, signed_in("tok-1", now() + 60));
    gh.runs("octo/app").await.unwrap();
    assert_eq!(fake.lock().unwrap().refreshes, 1);
    // and a 401 with a fresh token refreshes too
    let (fake, base) = start_fake().await;
    fake.lock().unwrap().valid = "tok-2".into();
    let (gh, _) = github(&base, signed_in("stale", 0));
    gh.runs("octo/app").await.unwrap();
    assert_eq!(fake.lock().unwrap().refreshes, 1);
    assert_eq!(gh.status()["state"], "ready");
}

#[tokio::test]
async fn failed_runs_notify_once_and_not_on_the_first_look() {
    let (fake, base) = start_fake().await;
    {
        let mut f = fake.lock().unwrap();
        f.valid = "tok-1".into();
        f.runs = vec![(1, "completed", "failure", "octo"), (2, "in_progress", "", "octo"), (3, "in_progress", "", "someone")];
    }
    let (gh, sent) = github(&base, signed_in("tok-1", 0));
    gh.runs("octo/app").await.unwrap(); // the first look: the old failure (run 1) is not news
    assert!(sent.lock().unwrap().is_empty());
    {
        let mut f = fake.lock().unwrap();
        f.runs = vec![(1, "completed", "failure", "octo"), (2, "completed", "failure", "octo"), (3, "completed", "failure", "someone"), (4, "completed", "success", "octo")];
        f.runs_etag += 1;
    }
    gh.poll_once().await;
    let got = sent.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "only my run that just failed: {got:?}");
    assert_eq!(got[0]["kind"], "run_failed");
    assert_eq!(got[0]["runId"], 2);
    assert_eq!(got[0]["route"], "github?repo=octo/app&run=2");
    // nothing new → nothing more (and the poller waits 60 s for an idle repo)
    gh.runs("octo/app").await.unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(gh.status()["recent"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn successes_notify_only_when_turned_on() {
    let (fake, base) = start_fake().await;
    {
        let mut f = fake.lock().unwrap();
        f.valid = "tok-1".into();
        f.runs = vec![(5, "queued", "", "octo")];
    }
    let (gh, sent) = github(&base, signed_in("tok-1", 0));
    gh.set_notify(None, Some(true)).unwrap();
    gh.runs("octo/app").await.unwrap();
    {
        let mut f = fake.lock().unwrap();
        f.runs = vec![(5, "completed", "success", "octo")];
        f.runs_etag += 1;
    }
    gh.runs("octo/app").await.unwrap();
    let got = sent.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["kind"], "run_succeeded");
}

#[tokio::test]
async fn run_detail_and_failed_log() {
    let (fake, base) = start_fake().await;
    fake.lock().unwrap().valid = "tok-1".into();
    let (gh, _) = github(&base, signed_in("tok-1", 0));
    let r = gh.run("octo/app", 9).await.unwrap();
    assert_eq!(r["jobs"][0]["id"], 90);
    assert_eq!(r["jobs"][0]["steps"][1]["conclusion"], "failure");
    let l = gh.log("octo/app", 90).await.unwrap();
    assert_eq!(l["lines"][1]["error"], true);
    assert_eq!(l["lines"][1]["text"], "Process completed with exit code 1.");
    assert!(gh.runs("../etc").await.is_err());
}
