//! GitHub, read-only (docs/flows/github.md). The person signs in with the device flow of the wridge-app GitHub
//! App; the server keeps the token in Windows Credential Manager and is the only one that talks to GitHub —
//! the window never sees a token. Reads use ETags (a 304 costs no rate limit). A background poller watches the
//! repos the person looked at and prints `[wridge:notify] {json}` when one of their runs ends, which the
//! desktop app turns into a Windows notification.
//!
//! Rust engine only. Tests point `WRIDGE_GITHUB_API` / `WRIDGE_GITHUB_WEB` at a fake GitHub and use the
//! in-memory store (`WRIDGE_GITHUB_STORE=memory`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The wridge-app GitHub App (public; device flow needs no secret).
pub const CLIENT_ID: &str = "Iv23liculOqtNY9MWvmv";
pub const APP_SLUG: &str = "wridge-app";
const POLL_TICK: Duration = Duration::from_secs(5);
const ACTIVE_EVERY_S: i64 = 5;
const IDLE_EVERY_S: i64 = 60;
const WATCH_MAX: usize = 5;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// What survives a restart: the tokens, who signed in, the notification choices and the watched repos.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub access_token: String,
    /// unix seconds; 0 = never expires
    pub expires_at: i64,
    pub refresh_token: String,
    pub refresh_expires_at: i64,
    pub login: String,
    #[serde(default = "yes")]
    pub notify_failed: bool,
    #[serde(default)]
    pub notify_succeeded: bool,
    /// repos the person looked at, newest first ("owner/name")
    #[serde(default)]
    pub watch: Vec<String>,
}
fn yes() -> bool {
    true
}

/// Where `Saved` lives.
pub trait Store: Send + Sync {
    fn load(&self) -> Option<Saved>;
    fn save(&self, s: &Saved) -> anyhow::Result<()>;
    fn clear(&self);
}

#[derive(Default)]
pub struct MemoryStore(pub Mutex<Option<Saved>>);
impl Store for MemoryStore {
    fn load(&self) -> Option<Saved> {
        self.0.lock().unwrap().clone()
    }
    fn save(&self, s: &Saved) -> anyhow::Result<()> {
        *self.0.lock().unwrap() = Some(s.clone());
        Ok(())
    }
    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

/// Windows Credential Manager (generic credential "wridge" / "github").
#[cfg(windows)]
pub struct KeyringStore;
#[cfg(windows)]
impl KeyringStore {
    fn entry() -> Option<keyring::Entry> {
        keyring::Entry::new("wridge", "github").ok()
    }
}
#[cfg(windows)]
impl Store for KeyringStore {
    fn load(&self) -> Option<Saved> {
        serde_json::from_str(&Self::entry()?.get_password().ok()?).ok()
    }
    fn save(&self, s: &Saved) -> anyhow::Result<()> {
        let e = Self::entry().ok_or_else(|| anyhow!("Windows Credential Manager is not available"))?;
        e.set_password(&serde_json::to_string(s)?).map_err(|e| anyhow!("couldn't save the GitHub sign-in: {e}"))
    }
    fn clear(&self) {
        if let Some(e) = Self::entry() {
            let _ = e.delete_credential();
        }
    }
}

pub struct Config {
    pub client_id: String,
    pub api: String,
    pub web: String,
}
impl Config {
    pub fn from_env() -> Config {
        let get = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        Config {
            client_id: get("WRIDGE_GITHUB_CLIENT_ID", CLIENT_ID),
            api: get("WRIDGE_GITHUB_API", "https://api.github.com").trim_end_matches('/').to_string(),
            web: get("WRIDGE_GITHUB_WEB", "https://github.com").trim_end_matches('/').to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct Signin {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_at: i64,
    interval: u64,
    /// pending | denied | expired | error: <message>
    state: String,
    /// bumped by cancel / a new sign-in so an old poll loop stops
    run: u64,
}

#[derive(Default)]
struct St {
    saved: Option<Saved>,
    signin: Option<Signin>,
    signin_run: u64,
    expired: bool,
    /// url → (etag, body)
    cache: HashMap<String, (String, Value)>,
    rate_remaining: Option<i64>,
    /// GitHub unreachable: (error, since)
    offline: Option<(String, i64)>,
    /// repo → run id → (status, conclusion)
    seen: HashMap<String, HashMap<u64, (String, String)>>,
    /// repo → last poll (unix s)
    polled: HashMap<String, i64>,
    /// the notifications printed, newest last (the window shows them; tests read them)
    recent: VecDeque<Value>,
}

pub type Notify = Box<dyn Fn(&Value) + Send + Sync>;

pub struct GitHub {
    cfg: Config,
    store: Box<dyn Store>,
    http: reqwest::Client,
    st: Mutex<St>,
    notify: Notify,
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

// ---------------------------------------------------------------- GraphQL (phase 2: PRs, branches, commits, issues, releases)
// One query per tab: what REST needs dozens of calls for (review state, checks, ahead/behind) comes back in one.

const PR_FIELDS: &str = "number title isDraft state createdAt mergedAt closedAt url author { login } mergedBy { login } headRefName baseRefName additions deletions comments { totalCount } reviewDecision reviewRequests(first: 10) { nodes { requestedReviewer { ... on User { login } ... on Team { name } } } } commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";

const Q_OVERVIEW: &str = "query Overview($owner: String!, $name: String!, $since: DateTime!, $reviewQ: String!, $assignedQ: String!) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
    pullRequests(states: OPEN) { totalCount }
    refs(refPrefix: \"refs/heads/\", first: 100) { totalCount nodes { target { ... on Commit { committedDate } } } }
    issues(states: OPEN) { totalCount }
    newIssues: issues(states: OPEN, filterBy: { since: $since }) { totalCount }
    latestRelease { name tagName createdAt url author { login } releaseAssets(first: 1) { nodes { name size } } }
  }
  review: search(query: $reviewQ, type: ISSUE, first: 5) { issueCount nodes { ... on PullRequest { number title createdAt author { login } } } }
  assigned: search(query: $assignedQ, type: ISSUE, first: 5) { issueCount nodes { ... on Issue { number title updatedAt } } }
}";

const Q_PULL: &str = "query Pull($owner: String!, $name: String!, $n: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $n) {
      number title state isDraft merged mergeable url createdAt author { login } headRefName baseRefName bodyText additions deletions changedFiles
      commits(last: 20) { totalCount nodes { commit { oid abbreviatedOid messageHeadline committedDate url author { name user { login } } statusCheckRollup { state } } } }
      checks: commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 40) { nodes {
        __typename
        ... on CheckRun { name status conclusion startedAt completedAt detailsUrl checkSuite { workflowRun { workflow { name } } } }
        ... on StatusContext { context state targetUrl }
      } } } } } }
      reviewRequests(first: 10) { nodes { requestedReviewer { ... on User { login } ... on Team { name } } } }
      latestReviews(first: 10) { nodes { author { login } state } }
      files(first: 30) { totalCount nodes { path additions deletions } }
    }
  }
}";

const Q_BRANCHES: &str = "query Branches($owner: String!, $name: String!, $base: String!) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name target { ... on Commit { messageHeadline committedDate author { name user { login } } statusCheckRollup { state } } } }
    refs(refPrefix: \"refs/heads/\", first: 100, orderBy: { field: TAG_COMMIT_DATE, direction: DESC }) { totalCount nodes {
      name
      target { ... on Commit { messageHeadline committedDate author { name user { login } } statusCheckRollup { state } } }
      compare(headRef: $base) { aheadBy behindBy }
      associatedPullRequests(first: 1, states: OPEN) { nodes { number } }
    } }
  }
}";

const Q_COMMITS: &str = "query Commits($owner: String!, $name: String!, $ref: String!) {
  repository(owner: $owner, name: $name) {
    refs(refPrefix: \"refs/heads/\", first: 100, orderBy: { field: TAG_COMMIT_DATE, direction: DESC }) { nodes { name } }
    ref(qualifiedName: $ref) { name target { ... on Commit { history(first: 40) { totalCount nodes {
      oid abbreviatedOid messageHeadline committedDate url author { name user { login } } statusCheckRollup { state }
    } } } } }
  }
}";

const Q_ISSUES: &str = "query Issues($owner: String!, $name: String!, $states: [IssueState!]) {
  repository(owner: $owner, name: $name) {
    open: issues(states: OPEN) { totalCount }
    closed: issues(states: CLOSED) { totalCount }
    list: issues(states: $states, first: 50, orderBy: { field: CREATED_AT, direction: DESC }) { nodes {
      number title createdAt closedAt url author { login } comments { totalCount }
      labels(first: 6) { nodes { name color } } assignees(first: 3) { nodes { login } }
    } }
  }
}";

const Q_RELEASES: &str = "query Releases($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    releases(first: 15, orderBy: { field: CREATED_AT, direction: DESC }) { totalCount nodes {
      name tagName isPrerelease isLatest isDraft createdAt url author { login } description tagCommit { abbreviatedOid }
      releaseAssets(first: 10) { nodes { name size downloadCount downloadUrl } }
    } }
  }
}";

fn pulls_query() -> String {
    format!("query Pulls($owner: String!, $name: String!, $states: [PullRequestState!]) {{
  repository(owner: $owner, name: $name) {{
    open: pullRequests(states: OPEN) {{ totalCount }}
    merged: pullRequests(states: MERGED) {{ totalCount }}
    closed: pullRequests(states: CLOSED) {{ totalCount }}
    list: pullRequests(states: $states, first: 40, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{ nodes {{ {PR_FIELDS} }} }}
    recent: pullRequests(states: MERGED, first: 5, orderBy: {{ field: UPDATED_AT, direction: DESC }}) {{ nodes {{ {PR_FIELDS} }} }}
  }}
}}")
}

fn owner_name(repo: &str) -> anyhow::Result<(String, String)> {
    if !valid_repo(repo) {
        bail!("not a repo: {repo}");
    }
    let (o, n) = repo.split_once('/').unwrap();
    Ok((o.to_string(), n.to_string()))
}

/// "owner/name" with GitHub's characters only.
pub fn valid_repo(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| p.map(|p| !p.is_empty() && p.len() <= 100 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))).unwrap_or(false);
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

impl GitHub {
    pub fn new(cfg: Config, store: Box<dyn Store>, notify: Notify) -> Arc<GitHub> {
        let saved = store.load();
        let http = reqwest::Client::builder()
            .user_agent(concat!("wridge/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("http client");
        Arc::new(GitHub { cfg, store, http, st: Mutex::new(St { saved, ..Default::default() }), notify })
    }

    /// The real thing: config from the environment, Credential Manager (memory with WRIDGE_GITHUB_STORE=memory or
    /// off Windows), notifications on stdout for the desktop app.
    pub fn from_env() -> Arc<GitHub> {
        let memory = std::env::var("WRIDGE_GITHUB_STORE").map(|v| v == "memory").unwrap_or(false);
        #[cfg(windows)]
        let store: Box<dyn Store> = if memory { Box::new(MemoryStore::default()) } else { Box::new(KeyringStore) };
        #[cfg(not(windows))]
        let store: Box<dyn Store> = {
            let _ = memory;
            Box::new(MemoryStore::default())
        };
        GitHub::new(Config::from_env(), store, Box::new(|n| println!("[wridge:notify] {}", n)))
    }

    fn persist(&self, s: &Saved) -> anyhow::Result<()> {
        self.store.save(s)?;
        self.st.lock().unwrap().saved = Some(s.clone());
        Ok(())
    }

    // ---------------------------------------------------------------- status and settings

    pub fn status(&self) -> Value {
        let st = self.st.lock().unwrap();
        let state = if let Some(si) = &st.signin {
            if si.state == "pending" { "signing_in" } else { "signed_out" }
        } else if st.saved.is_none() {
            "signed_out"
        } else if st.expired {
            "expired"
        } else if st.offline.is_some() {
            "offline"
        } else {
            "ready"
        };
        let signin = st.signin.as_ref().map(|s| json!({"userCode": s.user_code, "verificationUri": s.verification_uri, "expiresIn": (s.expires_at - now()).max(0), "state": s.state}));
        let saved = st.saved.as_ref();
        json!({
            "state": state,
            "login": saved.map(|s| s.login.clone()),
            "signin": signin,
            "notify": saved.map(|s| json!({"failed": s.notify_failed, "succeeded": s.notify_succeeded})),
            "watch": saved.map(|s| s.watch.clone()).unwrap_or_default(),
            "rateRemaining": st.rate_remaining,
            "offline": st.offline.as_ref().map(|(e, since)| json!({"error": e, "since": since})),
            "installUrl": format!("{}/apps/{APP_SLUG}/installations/new", self.cfg.web),
            "manageUrl": format!("{}/settings/installations", self.cfg.web),
            "recent": st.recent.iter().cloned().collect::<Vec<_>>(),
        })
    }

    pub fn set_notify(&self, failed: Option<bool>, succeeded: Option<bool>) -> anyhow::Result<Value> {
        let mut s = self.st.lock().unwrap().saved.clone().ok_or_else(|| anyhow!("Sign in to GitHub first"))?;
        if let Some(f) = failed {
            s.notify_failed = f;
        }
        if let Some(v) = succeeded {
            s.notify_succeeded = v;
        }
        self.persist(&s)?;
        Ok(self.status())
    }

    pub fn sign_out(&self) {
        self.store.clear();
        let mut st = self.st.lock().unwrap();
        *st = St { signin_run: st.signin_run + 1, ..Default::default() };
    }

    // ---------------------------------------------------------------- device flow

    pub async fn sign_in(self: &Arc<Self>) -> anyhow::Result<Value> {
        let res = self
            .http
            .post(format!("{}/login/device/code", self.cfg.web))
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(&[("client_id", &self.cfg.client_id)]))
            .send()
            .await
            .map_err(|e| anyhow!("Couldn't reach GitHub: {e}"))?;
        let v: Value = serde_json::from_str(&res.text().await?).map_err(|_| anyhow!("GitHub sent something unexpected"))?;
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            bail!("GitHub said: {}", v.get("error_description").and_then(Value::as_str).unwrap_or(e));
        }
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("GitHub's answer has no {k}"));
        let run = {
            let mut st = self.st.lock().unwrap();
            st.signin_run += 1;
            st.signin = Some(Signin {
                device_code: s("device_code")?,
                user_code: s("user_code")?,
                verification_uri: s("verification_uri")?,
                expires_at: now() + v.get("expires_in").and_then(Value::as_i64).unwrap_or(900),
                interval: v.get("interval").and_then(Value::as_u64).unwrap_or(5),
                state: "pending".into(),
                run: st.signin_run,
            });
            st.signin_run
        };
        let me = self.clone();
        tokio::spawn(async move { me.poll_device(run).await });
        Ok(self.status())
    }

    pub fn cancel_sign_in(&self) {
        let mut st = self.st.lock().unwrap();
        st.signin = None;
        st.signin_run += 1;
    }

    async fn poll_device(self: Arc<Self>, run: u64) {
        loop {
            let (device_code, interval, expires_at) = {
                let st = self.st.lock().unwrap();
                match &st.signin {
                    Some(s) if s.run == run && s.state == "pending" => (s.device_code.clone(), s.interval, s.expires_at),
                    _ => return,
                }
            };
            tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
            if now() > expires_at {
                self.end_sign_in(run, "expired");
                return;
            }
            let res = self
                .http
                .post(format!("{}/login/oauth/access_token", self.cfg.web))
                .header("Accept", "application/json")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(form(&[("client_id", &self.cfg.client_id), ("device_code", &device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")]))
                .send()
                .await;
            let Ok(res) = res else { continue }; // a network blip: try again next tick
            let Ok(v) = serde_json::from_str::<Value>(&res.text().await.unwrap_or_default()) else { continue };
            match v.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => continue,
                Some("slow_down") => {
                    let mut st = self.st.lock().unwrap();
                    if let Some(s) = st.signin.as_mut().filter(|s| s.run == run) {
                        s.interval = v.get("interval").and_then(Value::as_u64).unwrap_or(s.interval + 5);
                    }
                    continue;
                }
                Some("expired_token") => return self.end_sign_in(run, "expired"),
                Some("access_denied") => return self.end_sign_in(run, "denied"),
                Some(other) => return self.end_sign_in(run, &format!("error: {other}")),
                None => {}
            }
            match self.take_tokens(&v, None).await {
                Ok(()) => {
                    let mut st = self.st.lock().unwrap();
                    if st.signin.as_ref().map(|s| s.run) == Some(run) {
                        st.signin = None;
                    }
                }
                Err(e) => self.end_sign_in(run, &format!("error: {e}")),
            }
            return;
        }
    }

    fn end_sign_in(&self, run: u64, state: &str) {
        let mut st = self.st.lock().unwrap();
        if let Some(s) = st.signin.as_mut().filter(|s| s.run == run) {
            s.state = state.to_string();
        }
    }

    /// A token answer (device flow or refresh) → Saved; asks GitHub who this is when `login` is unknown.
    async fn take_tokens(&self, v: &Value, keep: Option<&Saved>) -> anyhow::Result<()> {
        let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("no access token in GitHub's answer"))?.to_string();
        let t = now();
        let mut s = keep.cloned().unwrap_or(Saved { notify_failed: true, ..Default::default() });
        s.access_token = access;
        s.expires_at = v.get("expires_in").and_then(Value::as_i64).map(|e| t + e).unwrap_or(0);
        if let Some(r) = v.get("refresh_token").and_then(Value::as_str) {
            s.refresh_token = r.to_string();
            s.refresh_expires_at = v.get("refresh_token_expires_in").and_then(Value::as_i64).map(|e| t + e).unwrap_or(0);
        }
        if s.login.is_empty() {
            let res = self.http.get(format!("{}/user", self.cfg.api)).bearer_auth(&s.access_token).header("Accept", "application/vnd.github+json").send().await?;
            let me: Value = serde_json::from_str(&res.text().await?)?;
            s.login = me.get("login").and_then(Value::as_str).unwrap_or("").to_string();
        }
        self.persist(&s)?;
        let mut st = self.st.lock().unwrap();
        st.expired = false;
        Ok(())
    }

    /// A usable access token, refreshed when it ends within 5 minutes (no client secret: device-flow tokens).
    async fn token(&self) -> anyhow::Result<String> {
        let saved = self.st.lock().unwrap().saved.clone().ok_or_else(|| anyhow!("Sign in to GitHub first"))?;
        if saved.expires_at == 0 || saved.expires_at - now() > 300 {
            return Ok(saved.access_token);
        }
        self.refresh(&saved).await?;
        Ok(self.st.lock().unwrap().saved.as_ref().map(|s| s.access_token.clone()).unwrap_or_default())
    }

    async fn refresh(&self, saved: &Saved) -> anyhow::Result<()> {
        let fail = |me: &Self| {
            me.st.lock().unwrap().expired = true;
            anyhow!("Your GitHub sign-in expired — sign in again")
        };
        if saved.refresh_token.is_empty() || (saved.refresh_expires_at != 0 && saved.refresh_expires_at < now()) {
            return Err(fail(self));
        }
        let res = self
            .http
            .post(format!("{}/login/oauth/access_token", self.cfg.web))
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(&[("client_id", &self.cfg.client_id), ("grant_type", "refresh_token"), ("refresh_token", &saved.refresh_token)]))
            .send()
            .await
            .map_err(|e| anyhow!("Couldn't reach GitHub: {e}"))?;
        let v: Value = serde_json::from_str(&res.text().await.unwrap_or_default()).unwrap_or(Value::Null);
        if v.get("access_token").is_none() {
            return Err(fail(self));
        }
        self.take_tokens(&v, Some(saved)).await
    }

    // ---------------------------------------------------------------- reading

    /// GET on the API with the cached ETag. Offline with a cached answer → that answer (and state "offline").
    async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.cfg.api, path);
        let mut tried_refresh = false;
        loop {
            let token = self.token().await?;
            let etag = self.st.lock().unwrap().cache.get(&url).map(|(e, _)| e.clone());
            let mut req = self.http.get(&url).bearer_auth(&token).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(e) = &etag {
                req = req.header("If-None-Match", e);
            }
            let res = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    let mut st = self.st.lock().unwrap();
                    if st.offline.is_none() {
                        st.offline = Some((e.to_string(), now()));
                    }
                    return st.cache.get(&url).map(|(_, v)| v.clone()).ok_or_else(|| anyhow!("Can't reach GitHub"));
                }
            };
            let status = res.status().as_u16();
            let remaining = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
            let new_etag = res.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
            {
                let mut st = self.st.lock().unwrap();
                st.offline = None;
                if remaining.is_some() {
                    st.rate_remaining = remaining;
                }
            }
            if status == 401 && !tried_refresh {
                tried_refresh = true;
                let saved = self.st.lock().unwrap().saved.clone();
                if let Some(s) = saved {
                    self.refresh(&s).await?;
                    continue;
                }
            }
            if status == 304 {
                if let Some((_, v)) = self.st.lock().unwrap().cache.get(&url) {
                    return Ok(v.clone());
                }
            }
            let text = res.text().await.unwrap_or_default();
            if !(200..300).contains(&status) {
                let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| format!("HTTP {status}"));
                bail!("GitHub: {msg}");
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if let Some(e) = new_etag {
                self.st.lock().unwrap().cache.insert(url, (e, v.clone()));
            }
            return Ok(v);
        }
    }

    /// The repos the person let wridge-app read, from every installation.
    pub async fn repos(&self) -> anyhow::Result<Value> {
        let inst = self.get("/user/installations?per_page=100").await?;
        let mut out = vec![];
        for i in inst.get("installations").and_then(Value::as_array).cloned().unwrap_or_default() {
            let Some(id) = i.get("id").and_then(Value::as_u64) else { continue };
            let r = self.get(&format!("/user/installations/{id}/repositories?per_page=100")).await?;
            for repo in r.get("repositories").and_then(Value::as_array).cloned().unwrap_or_default() {
                out.push(json!({
                    "fullName": repo["full_name"], "name": repo["name"], "owner": repo["owner"]["login"], "private": repo["private"],
                    "defaultBranch": repo["default_branch"], "url": repo["html_url"], "pushedAt": repo["pushed_at"],
                }));
            }
        }
        let watch = self.st.lock().unwrap().saved.as_ref().map(|s| s.watch.clone()).unwrap_or_default();
        Ok(json!({"repos": out, "watch": watch}))
    }

    fn remember(&self, repo: &str) {
        let s = self.st.lock().unwrap().saved.clone();
        if let Some(mut s) = s {
            if s.watch.first().map(String::as_str) == Some(repo) {
                return;
            }
            s.watch.retain(|r| r != repo);
            s.watch.insert(0, repo.to_string());
            s.watch.truncate(WATCH_MAX);
            let _ = self.persist(&s);
        }
    }

    fn run_json(r: &Value) -> Value {
        let short = r["head_sha"].as_str().map(|s| s.chars().take(7).collect::<String>());
        json!({
            "id": r["id"], "number": r["run_number"], "workflow": r["name"], "title": r["display_title"], "status": r["status"],
            "conclusion": r["conclusion"], "branch": r["head_branch"], "sha": short, "event": r["event"],
            "actor": r["actor"]["login"], "triggeredBy": r["triggering_actor"]["login"], "createdAt": r["created_at"],
            "startedAt": r["run_started_at"], "updatedAt": r["updated_at"], "url": r["html_url"], "attempt": r["run_attempt"],
        })
    }

    pub async fn runs(&self, repo: &str) -> anyhow::Result<Value> {
        if !valid_repo(repo) {
            bail!("not a repo: {repo}");
        }
        self.remember(repo);
        let v = self.get(&format!("/repos/{repo}/actions/runs?per_page=30")).await?;
        let list: Vec<Value> = v.get("workflow_runs").and_then(Value::as_array).cloned().unwrap_or_default();
        self.observe(repo, &list, false);
        Ok(json!({"repo": repo, "total": v["total_count"], "runs": list.iter().map(Self::run_json).collect::<Vec<_>>()}))
    }

    pub async fn run(&self, repo: &str, id: u64) -> anyhow::Result<Value> {
        if !valid_repo(repo) {
            bail!("not a repo: {repo}");
        }
        let r = self.get(&format!("/repos/{repo}/actions/runs/{id}")).await?;
        let j = self.get(&format!("/repos/{repo}/actions/runs/{id}/jobs?per_page=100")).await?;
        let jobs: Vec<Value> = j.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default().iter().map(|job| {
            json!({
                "id": job["id"], "name": job["name"], "status": job["status"], "conclusion": job["conclusion"], "startedAt": job["started_at"],
                "completedAt": job["completed_at"], "url": job["html_url"], "runner": job["runner_name"], "labels": job["labels"],
                "steps": job["steps"].as_array().cloned().unwrap_or_default().iter().map(|s| json!({
                    "number": s["number"], "name": s["name"], "status": s["status"], "conclusion": s["conclusion"], "startedAt": s["started_at"], "completedAt": s["completed_at"],
                })).collect::<Vec<_>>(),
            })
        }).collect();
        Ok(json!({"run": Self::run_json(&r), "jobs": jobs}))
    }

    /// The end of a finished job's log, around its first error (GitHub serves logs only after a job ends).
    pub async fn log(&self, repo: &str, job: u64) -> anyhow::Result<Value> {
        if !valid_repo(repo) {
            bail!("not a repo: {repo}");
        }
        let token = self.token().await?;
        let res = self
            .http
            .get(format!("{}/repos/{repo}/actions/jobs/{job}/logs", self.cfg.api))
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| anyhow!("Couldn't reach GitHub: {e}"))?;
        if !res.status().is_success() {
            return Ok(json!({"lines": [], "available": false}));
        }
        let text = res.text().await.unwrap_or_default();
        Ok(log_tail(&text))
    }

    // ---------------------------------------------------------------- phase 2: one GraphQL query per tab

    /// A GraphQL query. Its last answer is kept, so a dropped connection still shows what was seen before.
    async fn graphql(&self, key: &str, query: &str, vars: Value) -> anyhow::Result<Value> {
        let url = format!("{}/graphql", self.cfg.api);
        let ck = format!("graphql:{key}:{vars}");
        let mut tried_refresh = false;
        loop {
            let token = self.token().await?;
            let sent = self.http.post(&url).bearer_auth(&token).header("Accept", "application/vnd.github+json").header("Content-Type", "application/json").body(json!({"query": query, "variables": vars}).to_string()).send().await;
            let res = match sent {
                Ok(r) => r,
                Err(e) => {
                    let mut st = self.st.lock().unwrap();
                    if st.offline.is_none() {
                        st.offline = Some((e.to_string(), now()));
                    }
                    return st.cache.get(&ck).map(|(_, v)| v.clone()).ok_or_else(|| anyhow!("Can't reach GitHub"));
                }
            };
            let status = res.status().as_u16();
            let remaining = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
            {
                let mut st = self.st.lock().unwrap();
                st.offline = None;
                if remaining.is_some() {
                    st.rate_remaining = remaining;
                }
            }
            if status == 401 && !tried_refresh {
                tried_refresh = true;
                let saved = self.st.lock().unwrap().saved.clone();
                if let Some(s) = saved {
                    self.refresh(&s).await?;
                    continue;
                }
            }
            let text = res.text().await.unwrap_or_default();
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if !(200..300).contains(&status) {
                bail!("GitHub: {}", v.get("message").and_then(Value::as_str).unwrap_or(&format!("HTTP {status}")));
            }
            let data = v.get("data").cloned().unwrap_or(Value::Null);
            // partial answers (a field the app can't read) still show; only no data at all is an error
            if data.is_null() || data.get("repository").map(Value::is_null).unwrap_or(false) {
                let msg = v["errors"][0]["message"].as_str().unwrap_or("no answer");
                bail!("GitHub: {msg}");
            }
            self.st.lock().unwrap().cache.insert(ck, (String::new(), data.clone()));
            return Ok(data);
        }
    }

    pub async fn overview(&self, repo: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        self.remember(repo);
        let since = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
        let vars = json!({"owner": owner, "name": name, "since": since,
            "reviewQ": format!("repo:{repo} is:pr is:open review-requested:@me"), "assignedQ": format!("repo:{repo} is:issue is:open assignee:@me")});
        let d = self.graphql("overview", Q_OVERVIEW, vars).await?;
        Ok(json!({"repo": repo, "data": d}))
    }

    pub async fn pulls(&self, repo: &str, state: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let states = match state { "merged" => json!(["MERGED"]), "closed" => json!(["CLOSED"]), _ => json!(["OPEN"]) };
        let d = self.graphql("pulls", &pulls_query(), json!({"owner": owner, "name": name, "states": states})).await?;
        Ok(json!({"repo": repo, "state": state, "data": d}))
    }

    pub async fn pull(&self, repo: &str, n: u64) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql("pull", Q_PULL, json!({"owner": owner, "name": name, "n": n})).await?;
        if d["repository"]["pullRequest"].is_null() {
            bail!("no pull request #{n} in {repo}");
        }
        Ok(json!({"repo": repo, "data": d}))
    }

    pub async fn branches(&self, repo: &str, base: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let base = if base.is_empty() || base.contains("..") { "main" } else { base };
        let d = self.graphql("branches", Q_BRANCHES, json!({"owner": owner, "name": name, "base": base})).await?;
        Ok(json!({"repo": repo, "base": base, "data": d}))
    }

    pub async fn commits(&self, repo: &str, branch: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let branch = if branch.is_empty() || branch.contains("..") { "main" } else { branch };
        let d = self.graphql("commits", Q_COMMITS, json!({"owner": owner, "name": name, "ref": format!("refs/heads/{branch}")})).await?;
        Ok(json!({"repo": repo, "branch": branch, "data": d}))
    }

    pub async fn issues(&self, repo: &str, state: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let states = if state == "closed" { json!(["CLOSED"]) } else { json!(["OPEN"]) };
        let d = self.graphql("issues", Q_ISSUES, json!({"owner": owner, "name": name, "states": states})).await?;
        Ok(json!({"repo": repo, "state": state, "data": d}))
    }

    pub async fn releases(&self, repo: &str) -> anyhow::Result<Value> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql("releases", Q_RELEASES, json!({"owner": owner, "name": name})).await?;
        Ok(json!({"repo": repo, "data": d}))
    }

    // ---------------------------------------------------------------- watching and notifying

    /// Compares runs with what was seen before; the first look at a repo only remembers (no burst of old news).
    fn observe(&self, repo: &str, runs: &[Value], from_poller: bool) {
        let mut out = vec![];
        {
            let mut st = self.st.lock().unwrap();
            let me = st.saved.as_ref().map(|s| (s.login.clone(), s.notify_failed, s.notify_succeeded));
            let first = !st.seen.contains_key(repo);
            let seen = st.seen.entry(repo.to_string()).or_default();
            for r in runs {
                let Some(id) = r["id"].as_u64() else { continue };
                let status = r["status"].as_str().unwrap_or("").to_string();
                let conclusion = r["conclusion"].as_str().unwrap_or("").to_string();
                let before = seen.insert(id, (status.clone(), conclusion.clone()));
                if first || status != "completed" {
                    continue;
                }
                // ended since the last look (or appeared already finished after the first look)
                let ended_now = match &before {
                    Some((s, _)) => s != "completed",
                    None => true,
                };
                let Some((login, on_fail, on_ok)) = me.clone() else { continue };
                let mine = r["actor"]["login"].as_str() == Some(login.as_str()) || r["triggering_actor"]["login"].as_str() == Some(login.as_str());
                if !ended_now || !mine {
                    continue;
                }
                let failed = matches!(conclusion.as_str(), "failure" | "timed_out" | "startup_failure");
                let ok = conclusion == "success";
                if (failed && on_fail) || (ok && on_ok) {
                    let name = repo.split('/').nth(1).unwrap_or(repo);
                    out.push(json!({
                        "kind": if failed { "run_failed" } else { "run_succeeded" },
                        "repo": repo, "runId": id,
                        "title": format!("{} {} · {name}", r["name"].as_str().unwrap_or("Run"), if failed { "failed" } else { "passed" }),
                        "body": format!("{} · {}", r["display_title"].as_str().unwrap_or(""), r["head_branch"].as_str().unwrap_or("")),
                        "route": format!("github?repo={repo}&run={id}"),
                        "url": r["html_url"], "at": now(),
                    }));
                }
            }
            for n in &out {
                st.recent.push_back(n.clone());
                while st.recent.len() > 20 {
                    st.recent.pop_front();
                }
            }
            if from_poller {
                st.polled.insert(repo.to_string(), now());
            }
        }
        for n in &out {
            (self.notify)(n);
        }
    }

    fn active(&self, repo: &str) -> bool {
        self.st.lock().unwrap().seen.get(repo).map(|m| m.values().any(|(s, _)| s != "completed")).unwrap_or(false)
    }

    /// One round of the poller: every watched repo that is due (5 s while a run is going, 60 s otherwise).
    pub async fn poll_once(&self) {
        let (watch, ready) = {
            let st = self.st.lock().unwrap();
            (st.saved.as_ref().map(|s| s.watch.clone()).unwrap_or_default(), st.saved.is_some() && !st.expired)
        };
        if !ready {
            return;
        }
        let slow = self.st.lock().unwrap().rate_remaining.map(|r| r < 200).unwrap_or(false);
        for repo in watch {
            let last = self.st.lock().unwrap().polled.get(&repo).copied().unwrap_or(0);
            let mut every = if self.active(&repo) { ACTIVE_EVERY_S } else { IDLE_EVERY_S };
            if slow {
                every *= 4;
            }
            if now() - last < every {
                continue;
            }
            if let Ok(v) = self.get(&format!("/repos/{repo}/actions/runs?per_page=30")).await {
                let list: Vec<Value> = v.get("workflow_runs").and_then(Value::as_array).cloned().unwrap_or_default();
                self.observe(&repo, &list, true);
            } else {
                self.st.lock().unwrap().polled.insert(repo, now());
            }
        }
    }

    pub fn spawn_poller(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(POLL_TICK).await;
                me.poll_once().await;
            }
        });
    }

    /// Opens a github.com page in the default browser (the device page, the install page, a run).
    pub fn open(&self, url: &str) -> anyhow::Result<()> {
        if !(url.starts_with(&format!("{}/", self.cfg.web)) || url.starts_with("https://github.com/")) {
            bail!("only github.com pages");
        }
        // tests against a fake GitHub: say it instead of opening a browser
        if std::env::var("WRIDGE_GITHUB_OPEN").map(|v| v == "log").unwrap_or(false) {
            println!("[wridge:github-open] {url}");
            return Ok(());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).creation_flags(0x0800_0000).spawn()?;
        }
        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(url).spawn()?;
        #[cfg(all(unix, not(target_os = "macos")))]
        std::process::Command::new("xdg-open").arg(url).spawn()?;
        Ok(())
    }
}

/// GitHub job logs: "2026-11-07T08:00:00.0000000Z text". Keeps the text, marks error lines, and returns the
/// 30 lines before the first error and 10 after it — or the last 40 lines when nothing is marked as an error.
pub fn log_tail(text: &str) -> Value {
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            let l = l.trim_start_matches('\u{feff}');
            match l.split_once(' ') {
                Some((ts, rest)) if ts.len() >= 20 && ts.as_bytes()[4] == b'-' && ts.ends_with('Z') => rest.to_string(),
                _ => l.to_string(),
            }
        })
        .collect();
    let is_err = |l: &str| l.contains("##[error]") || l.starts_with("Error:") || l.contains(" ERR!") || l.contains("error[");
    let first = lines.iter().position(|l| is_err(l));
    let (from, to) = match first {
        Some(i) => (i.saturating_sub(30), (i + 11).min(lines.len())),
        None => (lines.len().saturating_sub(40), lines.len()),
    };
    let out: Vec<Value> = lines[from..to].iter().map(|l| json!({"text": l.replace("##[error]", ""), "error": is_err(l), "group": l.starts_with("##[group]")})).collect();
    json!({"lines": out, "available": true, "firstError": first.map(|i| i - from)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_are_checked() {
        assert!(valid_repo("Warayutkub/wridge"));
        assert!(valid_repo("a-b/c.d_e"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a"));
        assert!(!valid_repo("a/../b"));
        assert!(!valid_repo("a/b?x=1"));
    }

    #[test]
    fn log_tail_finds_the_error() {
        let mut text = String::new();
        for i in 0..50 {
            text.push_str(&format!("2026-11-07T08:00:{:02}.0000000Z line {i}\n", i % 60));
        }
        text.push_str("2026-11-07T08:01:00.0000000Z ##[error]Process completed with exit code 1.\n");
        text.push_str("2026-11-07T08:01:01.0000000Z after\n");
        let v = log_tail(&text);
        let lines = v["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 32);
        assert_eq!(lines[0]["text"], "line 20");
        assert_eq!(lines[30]["text"], "Process completed with exit code 1.");
        assert_eq!(lines[30]["error"], true);
        assert_eq!(v["firstError"], 30);
        // no error: the last 40
        let v = log_tail("a\nb\nc");
        assert_eq!(v["lines"].as_array().unwrap().len(), 3);
        assert_eq!(v["firstError"], Value::Null);
    }
}
