// wridge window: the sidebar, Overview and Settings drawn here; the other pages (notes, graphs, code…)
// come from the server and sit in a frame. Server start/stop/logs go through the Tauri commands.
(() => {
  // the server this window talks to: the installed one, or whichever served this page (a dev server on another port)
  const API = /^(127\.0\.0\.1|localhost):\d+$/.test(location.host) ? location.origin : 'http://127.0.0.1:3156';
  const T = window.__TAURI__;
  const { esc, toast, confirmDialog, setLoading } = ui;
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const store = {
    get: (k, d = null) => { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
    set: (k, v) => { try { localStorage.setItem(k, v); } catch { /* storage blocked */ } },
  };
  const ICON = (n) => window.ICON?.[n] ?? '';
  const ic = (n, cls = 'i') => `<span class="${cls}" data-i="${n}">${ICON(n)}</span>`;
  const STK = (mood) => `stickers/${mood}.png`;

  // Outside the app (a browser preview of this page) there are no Tauri commands: read the server instead.
  const invoke = T?.core?.invoke ?? (async (cmd) => {
    if (cmd === 'server_status') return { running: httpOk, pid: http?.pid ?? null, startedAt: http ? Math.round(http.startedAt / 1000) : null, lastExit: null, stoppedByUser: !httpOk, startError: null, wsPort: 3155, httpPort: 3156, url: API, mcpUrl: `${API}/mcp`, pluginDir: '(wridge)/figma-plugin', pluginBuilt: true };
    if (cmd === 'server_logs') return [];
    if (cmd === 'get_prefs') return { autoStartServer: true };
    throw new Error('Only in the wridge app');
  });
  const listen = (ev, fn) => T?.event?.listen(ev, (e) => fn(e.payload));

  // ---------- state ----------
  let S = null;             // the app's view of the server process (Rust)
  let http = null;          // GET /api/status while the server answers
  let httpOk = false;
  let projects = [];
  let project = store.get('wridge-project', '');
  let brain = null;         // { notes, links, broken, trash }
  let logs = [];
  let prefs = { autoStartServer: true };
  let route = 'overview';
  let frameUrl = '';
  let claude = null;        // Claude Code setup: { installed, mcp, hook: on|other|off, hookCommand, configFile, settingsFile }
  let figma = null;         // GET /api/sessions: the open Figma files, which one this window shows, who works in each
  let figmaPick = store.get('wridge-figma', ''); // the file key (or name, older plugins) picked here
  let figmaTried = '';      // a pick already asked for, so a poll does not ask again and again
  let shownFile = null;     // { key, name } this window showed last (to say when it closes)
  let appVersion = '';      // the wridge app's own version (Tauri); the server's is http.version
  let claudeAt = 0;
  const visited = new Set();

  const PAGES = {
    notes: { path: '/brain', title: 'Notes' }, graph: { path: '/brain', view: 'graph', title: 'Graph' }, trash: { path: '/brain', view: 'trash', title: 'Trash' },
    'design-map': { path: '/graph', title: 'Design map' }, motion: { path: '/motion', title: 'Motion' }, pdf: { path: '/pdf', title: 'PDF' },
    code: { path: '/code', title: 'Code graph' }, data: { path: '/data', title: 'Data' }, flows: { path: '/flows', title: 'Flows' }, skills: { path: '/skills', title: 'Skills' },
  };
  const routeOf = (path, search) => {
    if (path === '/brain') { const v = new URLSearchParams(search).get('view'); return v === 'graph' ? 'graph' : v === 'trash' ? 'trash' : 'notes'; }
    return Object.keys(PAGES).find((k) => PAGES[k].path === path && !PAGES[k].view) ?? null;
  };

  // overall state of the bridge
  function kind() {
    if (!S) return 'off';
    if (!S.running) return S.startError || (S.lastExit != null && S.lastExit !== 0 && !S.stoppedByUser) ? 'error' : 'stopped';
    if (!httpOk) return 'starting';
    return http?.pluginConnected ? 'ready' : 'waiting';
  }
  const serverUp = () => ['ready', 'waiting'].includes(kind());

  // ---------- small helpers ----------
  const base = (p) => String(p ?? '').replaceAll('\\', '/').replace(/\/$/, '').split('/').pop() || p;
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  const ago = (ms) => { const m = Math.round((Date.now() - ms) / 60000); return m < 1 ? 'just now' : m < 60 ? `${m} min ago` : m < 1440 ? `${Math.floor(m / 60)} h ago` : `${Math.floor(m / 1440)} d ago`; };
  const uptime = (sec) => { if (!sec) return ''; const m = Math.floor((Date.now() / 1000 - sec) / 60); return m < 1 ? 'UP <1M' : m < 60 ? `UP ${m}M` : `UP ${Math.floor(m / 60)}H ${m % 60}M`; };
  const hhmm = (t) => new Date(t).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  async function api(path, opts) {
    const res = await fetch(API + path, { cache: 'no-store', ...opts });
    if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? `HTTP ${res.status}`);
    return res.json();
  }
  const portError = () => {
    const line = [...logs].reverse().find((l) => /already in use|EADDRINUSE/i.test(l));
    const port = line?.match(/Port (\d+)|:(\d{4,5})\b/);
    return line ? { port: port?.[1] ?? port?.[2] ?? '', line } : null;
  };
  const lastMcp = () => http?.history?.find((h) => h.source === 'MCP');
  const aiActive = () => { const h = lastMcp(); return h && Date.now() - h.time < 15 * 60000; };

  // ---------- sidebar ----------
  $$('[data-i]').forEach((el) => { el.innerHTML = ICON(el.dataset.i); });
  const root = document.documentElement;
  const tip = $('#tip');
  function hideTip() { tip.hidden = true; }
  function setSidebar(collapsed) {
    if (collapsed) root.dataset.sidebar = 'collapsed'; else delete root.dataset.sidebar;
    store.set('wridge-sidebar', collapsed ? 'collapsed' : 'open');
    $('#sb-toggle').setAttribute('aria-label', collapsed ? 'Expand the sidebar' : 'Collapse the sidebar');
    hideTip();
  }
  $('#sb-toggle').onclick = () => setSidebar(root.dataset.sidebar !== 'collapsed');
  setSidebar(root.dataset.sidebar === 'collapsed');
  document.addEventListener('keydown', (e) => { if (e.ctrlKey && e.key === '\\') { e.preventDefault(); setSidebar(root.dataset.sidebar !== 'collapsed'); } });

  // names as tooltips while the sidebar is a rail of icons
  $('.sb').addEventListener('mouseover', (e) => {
    const el = e.target.closest('[data-tip]');
    if (!el || root.dataset.sidebar !== 'collapsed') return hideTip();
    const r = el.getBoundingClientRect();
    tip.textContent = el.dataset.tip;
    tip.hidden = false;
    tip.style.left = `${r.right + 10}px`;
    tip.style.top = `${r.top + r.height / 2 - tip.offsetHeight / 2}px`;
  });
  $('.sb').addEventListener('mouseleave', hideTip);

  function paintSidebar() {
    const k = kind();
    const st = $('#sb-status');
    const text = { ready: 'Ready · Figma connected', waiting: 'Waiting for Figma', starting: 'Starting…', error: "Couldn't start", stopped: 'Server stopped', off: 'Checking…' }[k];
    st.dataset.kind = k;
    st.dataset.tip = text;
    $('#sb-status-txt').textContent = text;
    $('#cnt-notes').textContent = brain ? brain.notes : '';
    $('#cnt-trash').textContent = brain?.trash ? brain.trash : '';
    const p = projects.find((x) => x.dir === project);
    $('#proj-av').textContent = p ? base(p.dir).slice(0, 1) : '·';
    $('#proj-name').textContent = p ? base(p.dir) : projects.length ? 'Pick a project' : 'No project';
    $('#proj-sub').textContent = !serverUp() ? (p ? 'offline' : 'start the server') : brain ? plural(brain.notes, 'note') : p ? 'no notes yet' : 'Add a folder';
    $('#proj-btn').dataset.tip = p ? base(p.dir) : 'Project';
    paintFigma();
    $('#sb-ver').textContent = appVersion || http?.version || '';
    // a book or a game the code graph cannot read: hide the code pages (unless you are on one)
    $('#grp-code').hidden = Boolean(p && !p.codeFiles && !['code', 'data', 'flows'].includes(route));
    $$('.sb-item').forEach((a) => (a.dataset.route === route ? a.setAttribute('aria-current', 'page') : a.removeAttribute('aria-current')));
  }

  // ---------- Figma file picker: what this window shows (its own session, so no AI moves) ----------
  const figPop = $('#fig-pop');
  const figFiles = () => figma?.files ?? [];
  const shown = () => figFiles().find((f) => f.showing) ?? null;
  const fileId = (f) => f.fileKey || f.fileName || '';
  const kindOf = (f) => (f.editorType === 'figjam' ? 'FigJam' : 'Figma');
  function whoLine(f) {
    if (f.busy || f.waiting) return `${f.ai || 1} AI working${f.waiting ? ` · ${f.waiting} waiting` : ''}`;
    if (f.ai) return `${f.ai} AI here`;
    return f.editorType === 'figjam' ? 'FigJam · no one' : 'no AI here';
  }
  function paintFigma() {
    const f = serverUp() ? shown() : null;
    const av = $('#fig-av');
    av.classList.toggle('none', !f);
    $('#fig-name').textContent = f ? f.fileName : 'No Figma file';
    $('#fig-sub').textContent = !serverUp() ? 'start the server' : f ? `${kindOf(f)} · ${whoLine(f)}` : 'Run the wridge plugin in Figma';
    $('#fig-btn').dataset.tip = f ? f.fileName : 'Figma file';
  }
  function closeFigPop() { figPop.hidden = true; $('#fig-btn').setAttribute('aria-expanded', 'false'); }
  $('#fig-btn').onclick = (e) => {
    e.stopPropagation();
    closePop();
    if (!figPop.hidden) return closeFigPop();
    const r = $('#fig-btn').getBoundingClientRect();
    const collapsed = root.dataset.sidebar === 'collapsed';
    figPop.style.left = `${collapsed ? r.right + 8 : r.left}px`;
    figPop.style.top = `${collapsed ? r.top : r.bottom + 6}px`;
    drawFigma();
    figPop.hidden = false;
    $('#fig-btn').setAttribute('aria-expanded', 'true');
  };
  function drawFigma() {
    const files = figFiles();
    const rows = files.map((f) => `<button class="proj-opt" role="option" aria-selected="${f.showing}" data-file="${esc(fileId(f))}">
        ${f.editorType === 'figjam' ? '<span class="fig-tile jam">J</span>' : `<span class="fig-tile">${ICON('logo-figma')}</span>`}
        <span><b>${esc(f.fileName ?? 'Untitled')}</b><small class="meta">${f.busy || f.waiting ? '<i data-s="busy"></i>' : f.ai ? '<i data-s="ai"></i>' : ''}${esc(f.showing && f.ai ? `${f.ai} AI session${f.ai === 1 ? '' : 's'} here` : whoLine(f))}</small></span>
        ${f.showing ? '<em class="showing">showing</em>' : ''}</button>`).join('');
    figPop.innerHTML = `<p class="label fig-head">◇ Open in Figma · ${files.length}</p>
      ${rows || `<p class="proj-none">${serverUp() ? 'No file yet.' : 'The server is stopped.'}</p>`}
      <hr><div class="fig-foot"><span>Another file? Open it in Figma and run Plugins → Development → wridge.</span>
      <span class="faint">Picking a file here changes only what this window shows — AI sessions keep their own file.</span></div>`;
  }
  figPop.onclick = (e) => {
    e.stopPropagation();
    const opt = e.target.closest('.proj-opt');
    if (!opt) return;
    closeFigPop();
    pickFigma(opt.dataset.file, true);
  };
  document.addEventListener('click', (e) => { if (!figPop.hidden && !figPop.contains(e.target)) closeFigPop(); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeFigPop(); });
  async function pickFigma(file, byHand) {
    if (!file) return;
    if (byHand) { figmaPick = file; store.set('wridge-figma', file); }
    figmaTried = file;
    try {
      figma = await api('/api/figma-file', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ file }) });
    } catch (err) { if (byHand) toast(`Couldn’t switch — ${err.message}`, { kind: 'error' }); return; }
    figmaChanged();
  }
  // after every read of /api/sessions: keep the picked file, say when it closed, reload a Figma page that now shows another file
  function figmaChanged() {
    const f = shown();
    const files = figFiles();
    const wanted = figmaPick && files.find((x) => fileId(x) === figmaPick || x.fileName === figmaPick);
    if (wanted && !wanted.showing && figmaTried !== figmaPick) { pickFigma(fileId(wanted), false); return; }
    const now = f ? { key: fileId(f), name: f.fileName } : null;
    if (shownFile && now && shownFile.key !== now.key) {
      if (!files.some((x) => fileId(x) === shownFile.key)) toast(`${shownFile.name} was closed in Figma — showing ${now.name}`);
      if (['design-map', 'motion', 'pdf'].includes(route)) loadPage(route, true);
    }
    if (now) shownFile = now;
    if (wanted?.showing) figmaTried = '';
    paintSidebar();
    if (!figPop.hidden) drawFigma();
  }

  // project picker
  const pop = $('#proj-pop');
  function closePop() { pop.hidden = true; $('#proj-btn').setAttribute('aria-expanded', 'false'); }
  $('#proj-btn').onclick = (e) => {
    e.stopPropagation();
    closeFigPop();
    if (!pop.hidden) return closePop();
    const r = $('#proj-btn').getBoundingClientRect();
    const collapsed = root.dataset.sidebar === 'collapsed';
    pop.style.left = `${collapsed ? r.right + 8 : r.left}px`;
    pop.style.top = `${collapsed ? r.top : r.bottom + 6}px`;
    pop.innerHTML = `<label class="proj-find">${ic('search')}<input type="search" placeholder="Find a project…" aria-label="Find a project" autocomplete="off" spellcheck="false"></label>
      <div class="proj-list" role="presentation"></div><hr>
      <button class="proj-opt proj-add" data-add>${ic('plus')}<span><b>Add a folder…</b></span></button>`;
    drawProjects('');
    pop.hidden = false;
    $('#proj-btn').setAttribute('aria-expanded', 'true');
    const input = $('.proj-find input', pop);
    input.oninput = () => drawProjects(input.value);
    input.onkeydown = (ev) => { if (ev.key === 'Enter') { const first = $('.proj-row .proj-opt', pop); if (first) { closePop(); choose(first.dataset.dir); } } };
    input.focus();
  };
  // the list part of the picker: filtered by name or path; each row can be removed from the list
  function drawProjects(q) {
    const list = $('.proj-list', pop);
    if (!list) return;
    const needle = q.trim().toLowerCase();
    const shown = projects.filter((p) => !needle || base(p.dir).toLowerCase().includes(needle) || p.dir.toLowerCase().includes(needle));
    list.innerHTML = shown.map((p) => `<div class="proj-row"><button class="proj-opt" role="option" aria-selected="${p.dir === project}" data-dir="${esc(p.dir)}"><span class="proj-av">${esc(base(p.dir).slice(0, 1))}</span><span><b>${esc(base(p.dir))}</b><small>${esc(p.dir)}</small></span></button><button class="proj-del" data-del="${esc(p.dir)}" aria-label="Remove ${esc(base(p.dir))} from the list" title="Remove from the list">${ICON('trash')}</button></div>`).join('')
      || `<p class="proj-none">${projects.length ? `No project matches “${esc(q)}”` : 'No projects yet — add a folder below.'}</p>`;
  }
  pop.onclick = async (e) => {
    e.stopPropagation();
    const del = e.target.closest('.proj-del');
    if (del) return removeProject(del.dataset.del);
    const opt = e.target.closest('.proj-opt');
    if (!opt) return;
    closePop();
    if (opt.dataset.add != null) return addProject();
    choose(opt.dataset.dir);
  };
  async function removeProject(dir) {
    closePop();
    const ok = await confirmDialog({ title: `Remove ${base(dir)} from the list?`, body: 'Only the list entry goes. The code and the project’s .wara/ folder stay, and you can add it back any time.', confirm: 'Remove', danger: true });
    if (!ok) return;
    try {
      await api('/api/code/projects/remove', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ dir }) });
    } catch (err) { return toast(`Couldn’t remove it — ${err.message}`, { kind: 'error' }); }
    await loadProjects();
    if (dir === project) choose(projects[0]?.dir ?? '');
    else paintSidebar();
    toast(`Removed ${base(dir)} from the list`);
  }
  document.addEventListener('click', (e) => { if (!pop.hidden && !pop.contains(e.target)) closePop(); });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closePop(); });
  function choose(dir) {
    project = dir;
    store.set('wridge-project', dir);
    brain = null;
    frameUrl = '';
    visited.clear();
    loadBrain();
    show(route, { force: true });
  }
  async function addProject() {
    if (!serverUp()) return toast('Start the server first', { kind: 'error' });
    try {
      const r = await api('/api/pick-folder', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ title: 'Add a project folder' }) });
      const dir = r.dir ?? r.path;
      if (!dir) return;
      await api('/api/code/projects', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ dir }) });
      await loadProjects();
      choose(projects.find((p) => base(p.dir) === base(dir))?.dir ?? dir);
      toast(`Added ${base(dir)}`);
    } catch (err) { toast(`Couldn't add the folder — ${err.message}`, { kind: 'error' }); }
  }

  // ---------- routing ----------
  const views = { overview: $('#v-overview'), settings: $('#v-settings'), page: $('#v-page') };
  function show(r, { force = false } = {}) {
    if (!PAGES[r] && r !== 'overview' && r !== 'settings') r = 'overview';
    const changed = r !== route;
    route = r;
    const key = PAGES[r] ? 'page' : r;
    for (const [k, v] of Object.entries(views)) {
      const on = k === key;
      if (on && v.hidden) { v.hidden = false; v.classList.remove('in'); void v.offsetWidth; v.classList.add('in'); }
      else if (!on) v.hidden = true;
    }
    if (r === 'overview') renderOverview(true);
    if (r === 'settings') renderSettings();
    if (PAGES[r]) loadPage(r, force || changed);
    paintSidebar();
    document.title = r === 'overview' ? 'wridge' : `${PAGES[r]?.title ?? 'Settings'} · wridge`;
  }
  const fromHash = () => (location.hash.replace(/^#\/?/, '') || 'overview').split('?')[0];
  window.addEventListener('hashchange', () => { if (fromHash() !== route) show(fromHash()); });

  // pages in the frame
  const frame = $('#frame');
  const loadBar = $('#page-load');
  function pageUrl(r) {
    const p = PAGES[r];
    const q = new URLSearchParams();
    if (project) q.set('dir', project);
    if (p.view) q.set('view', p.view);
    q.set('theme', root.dataset.theme);
    return `${API}${p.path}?${q}`;
  }
  function loadPage(r, reload) {
    const off = $('#page-off');
    if (!serverUp()) {
      off.hidden = false;
      off.innerHTML = pageOff(r);
      bindActions(off);
      return;
    }
    off.hidden = true;
    const url = pageUrl(r);
    if (!reload && frameUrl && routeOf(new URL(frameUrl).pathname, new URL(frameUrl).search) === r) return;
    frameUrl = url;
    loadBar.className = 'page-load on';
    frame.src = url;
  }
  frame.addEventListener('load', () => { loadBar.className = 'page-load done'; });
  // the page says where it is (a link inside it may have moved it): light the matching menu item
  window.addEventListener('message', (e) => {
    if (e.origin !== API) return;
    if (e.data?.type === 'wridge:navigate' && typeof e.data.route === 'string') { location.hash = `#/${e.data.route}`; return; }
    if (e.data?.type === 'wridge:save' && e.data.blob instanceof Blob) { saveForPage(e); return; }
    if (e.data?.type === 'wridge:reveal' && typeof e.data.path === 'string') { reveal(e.data.path); return; }
    if (e.data?.type !== 'wridge:path') return;
    const r = routeOf(e.data.path, e.data.search);
    if (!r) return;
    frameUrl = API + e.data.path + e.data.search;
    if (r !== route) { route = r; history.replaceState(null, '', `#/${r}`); paintSidebar(); }
  });
  // a page's download (PDF…): Windows' Save dialog asks where it goes; the page hears back the path, or null if cancelled
  async function saveForPage(e) {
    const { id, name, blob } = e.data;
    const reply = (msg) => e.source?.postMessage({ type: 'wridge:saved', id, ...msg }, e.origin);
    if (!T) return reply({ fallback: true }); // browser preview: a normal download
    try {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      const path = await invoke('save_as', bytes, { headers: { 'x-name': encodeURIComponent(name || 'download') } });
      reply({ path: path ?? null });
    } catch (err) { reply({ error: String(err?.message ?? err) }); }
  }
  const reveal = (path) => invoke('plugin:opener|reveal_item_in_dir', { paths: [path] }).catch(() => toast('Couldn’t open the folder. Copy the path above and paste it into File Explorer.', { kind: 'error' }));
  function pageOff(r) {
    const k = kind();
    const starting = k === 'starting';
    return `<div class="center">
      <div class="sticker" style="width:176px"><img src="${STK(starting ? 'walking' : k === 'error' ? 'stressed' : 'bored')}" alt=""></div>
      <p class="label">◇ ${esc(PAGES[r].title)}${project ? ` · ${esc(base(project))}` : ''}</p>
      <h2>${starting ? 'Starting the server…' : k === 'error' ? "The server couldn't start" : 'The server is stopped'}</h2>
      <p>${starting ? 'This page opens as soon as it answers — a few seconds.' : `${esc(PAGES[r].title)} lives in the server. Start it to open this page — it takes a few seconds.`}</p>
      <div class="row" style="padding-top:6px">${starting ? '<button class="btn btn-secondary is-loading" disabled><span class="spinner"></span>Starting</button>' : '<button class="btn btn-primary" data-act="start">Start server</button>'}<a class="btn btn-ghost" href="#/overview">Go to Overview</a></div>
      ${starting ? '' : '<p class="hint">Settings → Startup can start it when wridge opens</p>'}
    </div>`;
  }

  // ---------- server actions ----------
  async function act(btn, cmd, busy) {
    if (btn) setLoading(btn, true, busy);
    try { S = await invoke(cmd); httpOk = false; update(); }
    catch (err) { toast(String(err?.message ?? err), { kind: 'error' }); }
    finally { if (btn?.isConnected) setLoading(btn, false); }
    poll();
  }
  async function stopServer(btn) {
    const ok = await confirmDialog({ title: 'Stop the server?', body: 'Your AI and the Figma plugin can’t use wridge until you start it again.', confirm: 'Stop server', danger: true });
    if (ok) act(btn, 'server_stop', 'Stopping…');
  }
  function bindActions(scope) {
    $$('[data-act]', scope).forEach((b) => {
      b.onclick = () => {
        const a = b.dataset.act;
        if (a === 'start') act(b, 'server_start', 'Starting…');
        else if (a === 'restart') {
          const icon = b.querySelector('.i');
          if (icon) { icon.classList.remove('spin'); void icon.offsetWidth; icon.classList.add('spin'); setTimeout(() => icon.classList.remove('spin'), 900); }
          b.disabled = true;
          act(icon ? null : b, 'server_restart', 'Restarting…').finally(() => { b.disabled = false; });
        }
        else if (a === 'stop') stopServer(b);
        else if (a === 'copy') copy(b.dataset.text).then((ok) => {
          if (!ok) return;
          b.classList.remove('copied'); void b.offsetWidth; b.classList.add('copied');
          const t = b.querySelector('.t'); if (t) t.textContent = 'Copied';
          clearTimeout(b._t); b._t = setTimeout(() => { b.classList.remove('copied'); if (t) t.textContent = 'Copy'; }, 1600);
        });
        else if (a === 'folder') reveal(b.dataset.path);
        else if (a === 'claude-setup') claudeSetup(b);
        else if (a === 'claude-remove') claudeRemove(b);
        else if (a === 'auto-server') setPref(b);
        else if (a === 'engine') setEngine(b.dataset.engine);
        else if (a === 'theme') setTheme(b.dataset.theme);
        else if (a === 'clear-log') { logs = []; renderLogs(); }
      };
    });
  }
  async function copy(text) {
    try { await navigator.clipboard.writeText(text); return true; } catch { toast('Couldn’t copy — select the text instead', { kind: 'error' }); return false; }
  }
  // switch the server engine; a running server restarts on the new one
  async function setEngine(engine) {
    if ((prefs.serverEngine ?? 'rust') === engine) return;
    try {
      prefs = await invoke('set_prefs', { prefs: { ...prefs, serverEngine: engine } });
      renderSettings();
      if (S?.running) { toast(`Restarting on ${engine === 'node' ? 'Node' : 'Rust'}…`); await act(null, 'server_restart'); }
      else toast(`The server will start on ${engine === 'node' ? 'Node' : 'Rust'}`);
    } catch (err) { toast(`Couldn’t switch — ${err}`, { kind: 'error' }); }
  }
  async function setPref(btn) {
    const on = btn.getAttribute('aria-checked') !== 'true';
    try { prefs = await invoke('set_prefs', { prefs: { ...prefs, autoStartServer: on } }); btn.setAttribute('aria-checked', String(on)); toast(on ? 'The server starts when wridge opens' : 'You start the server by hand'); }
    catch (err) { toast(`Couldn’t save — ${err}`, { kind: 'error' }); }
  }

  // ---------- theme ----------
  const media = matchMedia('(prefers-color-scheme: dark)');
  function applyTheme() {
    const t = store.get('wridge-theme', 'system');
    const dark = t === 'dark' || (t === 'system' && media.matches);
    root.dataset.theme = dark ? 'dark' : 'light';
    frame.contentWindow?.postMessage({ type: 'wridge:theme', theme: root.dataset.theme }, '*');
    T?.window?.getCurrentWindow?.().setBackgroundColor?.(dark ? [20, 19, 17, 255] : [244, 242, 238, 255]).catch?.(() => {});
  }
  function setTheme(t) { store.set('wridge-theme', t); applyTheme(); if (route === 'settings') renderSettings(); }
  media.addEventListener('change', applyTheme);

  // ---------- overview ----------
  let lastSig = '';
  function renderOverview(force = false) {
    const k = kind();
    const sig = JSON.stringify([k, S?.pid, Math.floor(Date.now() / 60000), http?.pluginConnected, http?.fileName, figFiles().length, shown()?.fileName, http?.history?.slice(0, 5).map((h) => h.time), brain, project, k === 'stopped' || k === 'error' ? logs.slice(-12) : 0, prefs.autoStartServer]);
    if (!force && sig === lastSig) return;
    lastSig = sig;
    const v = views.overview;
    const keepScroll = v.scrollTop;
    const p = projects.find((x) => x.dir === project);
    const err = portError();
    const H = {
      ready: { label: `SERVER · ${S?.engine === 'node' ? 'NODE' : 'RUST'} · PID ${S?.pid ?? '–'} · ${uptime(S?.startedAt)}`, title: 'Ready', sub: aiActive() ? 'AI and Figma are both connected. Everything you do is remembered in this project’s brain.' : 'Figma is connected. Your AI joins on its own the next time it calls wridge.', actions: `<button class="btn btn-secondary" data-act="restart">${ic('restart')}Restart</button><button class="btn btn-ghost" data-act="stop">Stop</button>`, sticker: 'working' },
      waiting: { label: `SERVER · ${S?.engine === 'node' ? 'NODE' : 'RUST'} · PID ${S?.pid ?? '–'} · ${uptime(S?.startedAt)}`, title: 'Waiting for Figma', sub: 'The server is up. Open a design file and run the plugin to finish the bridge.', actions: `<a class="btn btn-primary" href="#/settings">How to connect</a><button class="btn btn-ghost" data-act="restart">${ic('restart')}Restart</button>`, sticker: 'phone' },
      starting: { label: 'SERVER · STARTING', title: 'Starting…', sub: 'Opening the ports and loading this project’s brain. This takes a few seconds.', actions: '<button class="btn btn-secondary is-loading" disabled><span class="spinner"></span>Starting</button>', sticker: 'walking', pulse: true },
      stopped: { label: 'SERVER · STOPPED', title: 'Stopped', sub: 'The server is off. Figma and your AI can’t reach this project’s brain until you start it.', actions: '<button class="btn btn-primary" data-act="start">Start server</button>', sticker: 'sleeping' },
      error: { label: `SERVER · FAILED${S?.lastExit != null ? ` · EXIT ${S.lastExit}` : ''}`, title: 'Couldn’t start', sub: err ? `Port ${err.port} is already in use, so wridge has nowhere to listen.` : S?.startError ? 'Node.js wasn’t found, so the server can’t run.' : 'The server stopped on its own. The log below says why.', actions: '<button class="btn btn-primary" data-act="start">Try again</button><a class="btn btn-secondary" href="#/settings">Open Settings</a>', sticker: 'stressed' },
      off: { label: 'SERVER', title: 'Checking…', sub: '', actions: '', sticker: 'thinking' },
    }[k];
    let html = `<div class="pad">
      <div class="hero${k === 'error' ? ' err' : ''}" data-k="${k}">
        <div class="hero-text">
          <p class="label">◇ ${esc(H.label)}</p>
          <h1><span class="light${H.pulse ? ' pulse' : ''}"></span>${esc(H.title)}</h1>
          <p>${esc(H.sub)}</p>
          <div class="actions">${H.actions}</div>
        </div>
        <div class="sticker"><img src="${STK(H.sticker)}" alt=""></div>
      </div>`;
    if (k === 'error' && (err || S?.startError)) {
      html += `<div class="banner"><div><b>${err ? `Port ${esc(err.port)} is in use` : 'Node.js is missing'}</b><p>${err ? 'Another program holds it — often an older wridge or wara-bridge still running. Stop it, then try again.' : 'Install Node.js (version 20 or newer), then try again.'}</p></div></div>`;
    }
    // connections
    const up = serverUp();
    const aiS = up ? (aiActive() ? 'connected' : 'waiting') : 'off';
    const figS = up ? (http?.pluginConnected ? 'connected' : 'waiting') : 'off';
    const mcp = lastMcp();
    html += `<p class="label">◇ Connections</p>
      <div class="bridge">
        ${node('AI', 'logo-claude', 30, 'Your AI', aiS === 'connected' ? `mcp · ${ago(mcp.time)}` : up ? (mcp ? `idle · ${ago(mcp.time)}` : 'mcp · :3156/mcp') : 'not connected', aiS === 'connected')}
        ${link(aiS)}
        ${node('wridge', 'logo-wridge', 40, `Brain${p ? ` · ${base(p.dir)}` : ''}`, up ? (brain ? `${plural(brain.notes, 'note')} · ${plural(brain.links, 'link')}` : p ? 'reading…' : 'no project yet') : k === 'starting' ? 'loading…' : 'offline', up || k === 'starting')}
        ${link(figS)}
        ${node('Figma', 'logo-figma', 22, http?.pluginConnected ? 'Figma' : 'Not connected', http?.pluginConnected ? `plugin · ${plural(figFiles().length || 1, 'file')} · ws :3155` : 'run the plugin', Boolean(http?.pluginConnected))}
      </div>
      <div class="rule"></div>`;
    if (k === 'starting') {
      const steps = [['done', 'Server process started', S?.engine === 'node' ? 'Node.js · legacy server' : 'Rust · wridge-server'], [httpOk ? 'done' : 'now', 'Port 3156 open', 'Pages and MCP · HTTP'], ['todo', 'Port 3155 open', 'Figma plugin · WebSocket'], ['todo', 'Waiting for Figma and your AI', 'They connect on their own']];
      html += `<p class="label">◇ Startup</p><div class="checklist">${steps.map(([s, t, d]) => `<div class="check" data-s="${s}"><span class="mark">${s === 'done' ? '✓' : ''}</span><div><b>${esc(t)}</b><small>${esc(d)}</small></div></div>`).join('')}</div>`;
    } else if (k === 'stopped' || k === 'error') {
      html += `<div class="two" style="flex:1;min-height:240px">
        ${logBox(k === 'error' ? 'Log' : 'Last session', 12)}
        ${k === 'stopped' ? `<div class="hint-card">
          <div class="hint-row"><span class="tile">${ic('startup')}</span><div class="grow"><b>Start the server when wridge opens</b><small>${prefs.autoStartServer ? 'On — it starts with the app.' : 'Off — you start it by hand.'}</small></div><button class="tgl" role="switch" aria-checked="${prefs.autoStartServer}" aria-label="Start the server when wridge opens" data-act="auto-server"${T ? '' : ' disabled'}></button></div>
          <div class="rule"></div>
          <div class="hint-row"><span class="tile">${ic('power')}</span><div class="grow"><b>Closing the window keeps it running</b><small>Stop it from here or from the tray menu.</small></div></div>
        </div>` : ''}
      </div>`;
    } else if (up) {
      const notesSub = brain ? `${plural(brain.notes, 'note')}${brain.broken ? ` · ${plural(brain.broken, 'broken link')}` : ''}` : 'Notes and links';
      html += `<p class="label">◇ Jump to</p><div class="cards">
        ${card('notes', 'notes', 'Notes', notesSub)}
        ${card('design-map', 'designmap', 'Design map', shown() ? `Figma · ${shown().fileName}` : 'Open a file in Figma')}
        ${card('code', 'code', 'Code graph', p ? (p.codeFiles ? plural(p.codeFiles, 'file') : 'No code read here') : 'Pick a project')}
        ${card('motion', 'motion', 'Motion', 'Lottie and motion tokens')}
      </div>
      <div class="cols">
        <div style="display:flex;flex-direction:column;gap:10px">
          <p class="label">◇ Brain${p ? ` · ${esc(base(p.dir))}` : ''}</p>
          <div class="stats">
            ${stat('notes', brain?.notes, 'Notes')}${stat('link', brain?.links, 'Links')}${stat('brokenlink', brain?.broken, 'Broken links', brain?.broken > 0)}${stat('trash', brain?.trash, 'In trash')}
          </div>
        </div>
        <div>
          <div class="act-head"><p class="label">◇ Recent activity</p><span class="mono" style="font-size:11px;color:var(--text-faint)">${http?.today ? `${plural(http.today.count, 'call')} today` : ''}</span></div>
          ${activity()}
        </div>
      </div>`;
    }
    html += '</div>';
    v.innerHTML = html;
    v.scrollTop = keepScroll;
    bindActions(v);
    if (k === 'stopped' || k === 'error') renderLogs();
  }
  const node = (key, logo, size, title, sub, on) => `<div class="node${on ? '' : ' off'}"><span class="logo" data-logo="${logo}"><span style="width:${size}px;display:block">${ICON(logo)}</span></span><div><p class="label">◇ ${esc(key)}</p><b>${esc(title)}</b><small>${esc(sub)}</small></div></div>`;
  const link = (s) => `<div class="link" data-s="${s}"><span class="line"></span><span class="pill" data-s="${s}">${s}</span><span class="line"></span></div>`;
  const card = (r, icon, title, sub) => `<a class="card-link" href="#/${r}"><span class="top"><span class="tile">${ic(icon)}</span>${ic('external')}</span><span><b>${esc(title)}</b><small>${esc(sub)}</small></span></a>`;
  const stat = (icon, n, label, bad) => `<div class="stat-cell${bad ? ' bad' : ''}">${ic(icon)}<div><div class="num">${n ?? '–'}</div><small>${esc(label)}</small></div></div>`;
  function activity() {
    const rows = (http?.history ?? []).slice(0, 5);
    if (!rows.length) return '<div class="empty-note">No calls yet. Ask your AI to do something in Figma or in the brain.</div>';
    return rows.map((h) => `<div class="act-row${h.ok ? '' : ' bad'}"><time>${hhmm(h.time)}</time><span class="src">${ICON(h.source === 'MCP' ? 'logo-claude' : 'logo-wridge')}</span><span class="tool grow">${esc(h.tool)}${h.ok ? '' : ' · failed'}</span><span class="ms">${h.ms} ms</span></div>`).join('');
  }
  const logBox = (title, n) => `<div class="logbox"><div class="logbox-head"><p class="label">◇ ${esc(title)}</p><a class="linkbtn" href="#/settings">${ic('terminal')}Full log</a></div><pre class="log" data-n="${n}"></pre></div>`;
  const lineHtml = (t) => `<span class="${/^—/.test(t) ? 'sys' : /error|failed|already in use|EADDRINUSE|couldn/i.test(t) ? 'err' : ''}">${esc(t)}</span>`;
  function renderLogs() {
    $$('pre.log').forEach((pre) => {
      const n = Number(pre.dataset.n) || 600;
      const stick = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 20;
      pre.innerHTML = logs.slice(-n).map(lineHtml).join('') || '<span class="sys">— nothing yet</span>';
      if (stick || n < 50) pre.scrollTop = pre.scrollHeight;
    });
  }

  // ---------- settings ----------
  function renderSettings() {
    const v = views.settings;
    const t = store.get('wridge-theme', 'system');
    const manifest = S ? `${S.pluginDir}/manifest.json` : '';
    const up = serverUp();
    const sec = (key, tile, title, desc, body) => `<section class="set" id="set-${key}"><aside>${tile}<p class="label">◇ ${esc(key)}</p><h3>${esc(title)}</h3><p>${esc(desc)}</p></aside><div class="set-body">${body}</div></section>`;
    const logoTile = (logo, size) => `<span class="tile lg"><span style="width:${size}px;display:block;color:var(--ink)">${ICON(logo)}</span></span>`;
    const copyf = (text) => `<div class="copyf"><code title="${esc(text)}">${esc(text)}</code><button class="btn btn-secondary" data-act="copy" data-text="${esc(text)}"><span class="copy-ic"><span class="sheets" style="display:contents">${ICON('copy')}</span><svg class="ok" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="M5 10.5l3.2 3.2L15 7" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg></span><span class="t">Copy</span></button></div>`;
    const theme = (id, name, halves) => `<button class="theme" data-act="theme" data-theme="${id}" aria-pressed="${t === id}"><span class="thumb">${halves.map((h) => `<span class="half ${h}"><span class="s"><i></i><i></i><i></i><i></i></span><span class="m"><i></i><em></em></span></span>`).join('')}</span>${name}</button>`;
    v.innerHTML = `<div class="pad">
      <div class="head"><p class="label">◇ Settings</p><h1>Settings</h1><p>How wridge connects to Figma and your AI, how it looks, and how it starts.</p></div>
      ${sec('Appearance', `<span class="tile lg"><svg viewBox="0 0 20 20" width="20" height="20" fill="none"><circle cx="10" cy="10" r="7.25" stroke="currentColor" stroke-width="1.5"/><path d="M10 2.75a7.25 7.25 0 0 1 0 14.5z" fill="currentColor"/></svg></span>`, 'Appearance', 'Light, dark, or follow Windows.',
        `<div class="themes">${theme('light', 'Light', ['l'])}${theme('dark', 'Dark', ['d'])}${theme('system', 'System', ['l', 'd'])}</div><p class="mono" style="font-size:11px;color:var(--text-faint);margin:0">Pages inside wridge follow the theme too.</p>`)}
      ${sec('Figma plugin', logoTile('logo-figma', 15), 'Figma plugin', 'Import it once. Updates of wridge replace the files in place.',
        `<div class="row"><span class="pill" data-s="${http?.pluginConnected ? 'connected' : up ? 'waiting' : 'off'}">${http?.pluginConnected ? 'connected' : up ? 'waiting' : 'server off'}</span>${shown() ? `<span class="mono" style="font-size:12px;color:var(--text-muted)">Showing: ${esc(shown().fileName)}${figFiles().length > 1 ? ` · ${figFiles().length} files open` : ''}</span>` : ''}</div>
         <div class="steps">${['In Figma: Plugins → Development → Import plugin from manifest…', 'Pick manifest.json from the folder below.', 'Open a design file and run Plugins → Development → wridge.'].map((s, i) => `<div class="step"><span class="n">${i + 1}</span>${esc(s)}</div>`).join('')}</div>
         ${manifest ? copyf(manifest) : ''}
         ${S && !S.pluginBuilt ? '<p style="color:var(--warning-text)">The plugin isn’t built yet (no code.js) — run <code>npm run build</code> in figma-plugin.</p>' : ''}
         <div><button class="linkbtn" data-act="folder" data-path="${esc(manifest)}" style="color:var(--text)">${ic('folder')}Show in folder</button></div>`)}
      ${sec('AI', logoTile('logo-claude', 22), 'Connect an AI', 'Claude Code and other MCP clients talk to wridge over Streamable HTTP.',
        `${claudeCard()}<p>Or by hand — Claude Code</p>${copyf(`claude mcp add --transport http --scope user wridge ${S?.mcpUrl ?? `${API}/mcp`}`)}<p>Any MCP client</p>${copyf(S?.mcpUrl ?? `${API}/mcp`)}`)}
      ${sec('Startup', `<span class="tile lg">${ic('startup')}</span>`, 'Startup', 'wridge lives in the tray. Closing the window keeps the server running.',
        `<div class="tgl-row"><div class="grow"><b>Start with Windows</b><small>Opens quietly in the tray when you sign in.</small></div><button class="tgl" role="switch" id="autostart" aria-checked="false" aria-label="Start with Windows"${T ? '' : ' disabled'}></button></div>
         <div class="tgl-row"><div class="grow"><b>Start the server when wridge opens</b><small>Turn off to start it by hand.</small></div><button class="tgl" role="switch" aria-checked="${prefs.autoStartServer}" aria-label="Start the server when wridge opens" data-act="auto-server"${T ? '' : ' disabled'}></button></div>`)}
      ${sec('Server', `<span class="tile lg">${ic('server')}</span>`, 'Server', 'Runs on this computer only. Nothing leaves it.',
        `<div class="tgl-row"><div class="grow"><b>Engine</b><small>${S?.engine === 'node' ? 'Node.js — the original server, kept as a fallback.' : 'Rust — one program, no Node.js needed.'}${S && !S.rustAvailable ? ' The Rust build is missing, so Node is used.' : ''}</small></div><div class="seg" role="group" aria-label="Server engine"><button data-act="engine" data-engine="rust" aria-pressed="${(prefs.serverEngine ?? 'rust') !== 'node'}"${S && !S.rustAvailable ? ' disabled' : ''}>Rust</button><button data-act="engine" data-engine="node" aria-pressed="${prefs.serverEngine === 'node' && S?.nodeAvailable !== false}"${S && !S.nodeAvailable ? ' disabled title="Only the development build has the Node server"' : ''}>Node</button></div></div>
         ${[['overview', 'Pages', API], ['plug', 'Figma plugin', 'ws://localhost:3155'], ['terminal', 'MCP', `${API}/mcp`]].map(([i, k, val]) => `<div class="kv-row">${ic(i)}<span class="k">${k}</span><code>${esc(val)}</code></div>`).join('')}
         <div class="row" style="padding-top:6px">${S?.running ? `<button class="btn btn-secondary" data-act="restart">${ic('restart')}Restart server</button><button class="btn btn-danger" data-act="stop">Stop server</button>` : '<button class="btn btn-primary" data-act="start">Start server</button>'}</div>
         <div class="logbox set-log"><div class="logbox-head"><p class="label">◇ Log</p><button class="linkbtn" data-act="clear-log">Clear</button></div><pre class="log" data-n="600"></pre></div>`)}
      ${sec('About', logoTile('logo-wridge', 24), 'About', 'Which wridge this is. Send these lines along when something breaks.', aboutBody())}
    </div>`;
    bindActions(v);
    renderLogs();
    refreshClaude();
    const auto = $('#autostart', v);
    if (T) {
      invoke('plugin:autostart|is_enabled').then((on) => auto.setAttribute('aria-checked', String(on))).catch(() => { auto.disabled = true; });
      auto.onclick = async () => {
        const on = auto.getAttribute('aria-checked') !== 'true';
        try { await invoke(on ? 'plugin:autostart|enable' : 'plugin:autostart|disable'); auto.setAttribute('aria-checked', String(on)); toast(on ? 'wridge starts with Windows' : 'wridge won’t start with Windows'); }
        catch (err) { toast(`Couldn’t change it — ${err}`, { kind: 'error' }); }
      };
    }
  }

  // About: the app, the server it runs, and the plugin in each open file; a server older than the app needs a restart
  function aboutBody() {
    const server = http?.version ?? '';
    const engine = S?.engine === 'node' ? 'Node engine' : 'Rust engine';
    const older = appVersion && server && server !== appVersion;
    const plugins = [...new Set(figFiles().map((f) => f.pluginVersion || 'older plugin'))];
    const row = (k, v, chip = '') => `<div class="kv-row"><span class="k">${esc(k)}</span><code>${esc(v)}</code>${chip}</div>`;
    return `<div class="about">
      ${older ? `<div class="warn"><b>The server is older than the app. Restart it (Server → Restart server) to use ${esc(appVersion)}.</b>The app updated, but the server it runs is still the old one.</div>` : ''}
      ${row('App', appVersion || '—')}
      ${row('Server', server ? `${server} · ${engine}` : serverUp() ? '—' : 'stopped', server && appVersion ? `<span class="pill" data-s="${older ? 'waiting' : 'connected'}">${older ? 'Older than the app' : 'Matches'}</span>` : '')}
      ${row('Figma plugin', figFiles().length ? `${plugins.join(', ')} · in ${plural(figFiles().length, 'file')}` : 'not open')}
      ${row('Installed in', S?.pluginDir ? S.pluginDir.replace(/[\\/]figma-plugin$/, '') : '—')}
    </div>`;
  }

  // ---------- Claude Code: one button adds the MCP server and the session-memory hook (user-wide) ----------
  const tilde = (p) => p?.replace(/^[A-Za-z]:\/Users\/[^/]+/, '~') ?? '';
  function claudeCard() {
    if (!T) return '<p class="mono" style="font-size:11px;color:var(--text-faint);margin:0">The one-click setup works in the wridge app.</p>';
    if (!claude) return '<div class="cc-card"><div class="cc-head"><div class="grow"><b>Claude Code</b><small>Checking…</small></div></div></div>';
    const done = claude.mcp === 'on' && claude.hook === 'on';
    const pill = (s) => `<span class="pill" data-s="${s === 'on' ? 'connected' : s === 'other' ? 'waiting' : 'off'}">${s === 'on' ? 'on' : s === 'other' ? 'elsewhere' : 'off'}</span>`;
    const hookNote = claude.hook === 'on' ? 'project memory at each session start'
      : claude.hook === 'other' ? (/wara-bridge/i.test(claude.hookCommand ?? '') ? 'runs wara-bridge — set up switches it to wridge' : 'runs another program — set up switches it to wridge')
      : 'not set';
    const sub = done ? 'Set up. wridge’s tools and project memory reach every Claude Code session.'
      : !claude.installed ? 'Claude Code isn’t on this computer yet. Set it up anyway — it’s ready when you install it.'
      : 'Lets Claude Code use wridge in every project, and remember each project between sessions.';
    return `<div class="cc-card${done ? ' done' : ''}">
      <div class="cc-head"><div class="grow"><b>Claude Code</b><small>${esc(sub)}</small></div>
        ${done ? `<button class="linkbtn" data-act="claude-remove">Remove</button>` : `<button class="btn btn-primary" data-act="claude-setup">Set up Claude Code</button>`}</div>
      <div class="cc-row">${pill(claude.mcp)}<span class="k">MCP server</span><code title="${esc(claude.configFile)}">${claude.mcp === 'on' ? `wridge · ${esc((S?.mcpUrl ?? `${API}/mcp`).replace(/^https?:\/\//, ''))}` : claude.mcp === 'other' ? 'wridge points to another address' : 'not added'}</code></div>
      <div class="cc-row">${pill(claude.hook)}<span class="k">Session memory</span><code title="${esc(claude.hookCommand ?? claude.settingsFile)}">${esc(hookNote)}</code></div>
    </div>`;
  }
  function repaintSettings() {
    if (route !== 'settings') return;
    const y = views.settings.scrollTop; renderSettings(); views.settings.scrollTop = y;
  }
  // read the two config files again (the user may have changed them by hand); repaint only when something changed
  function refreshClaude() {
    if (!T || Date.now() - claudeAt < 3000) return;
    claudeAt = Date.now();
    invoke('claude_status').then((c) => { if (JSON.stringify(c) !== JSON.stringify(claude)) { claude = c; repaintSettings(); } }).catch(() => {});
  }
  // back from another app (a terminal, an editor): the files may have changed meanwhile
  window.addEventListener('focus', () => { if (route === 'settings') { claudeAt = 0; refreshClaude(); } });
  async function claudeSetup(b) {
    const c = claude;
    const ok = await confirmDialog({
      title: 'Set up Claude Code?',
      body: `Changes two files, for every project. ${tilde(c?.configFile) || '~/.claude.json'} gets the wridge MCP server. ${tilde(c?.settingsFile) || '~/.claude/settings.json'} gets a hook that hands the AI the project’s memory when a session starts${c?.hook === 'other' ? ' — your current memory hook is pointed to wridge instead of adding a second one' : ''}. Everything else in them stays as it is.`,
      confirm: 'Set up',
    });
    if (!ok) return;
    setLoading(b, true, 'Setting up…');
    try {
      claude = await invoke('claude_setup');
      claudeAt = Date.now();
      toast('Claude Code is set up — start a new session to use it');
      window.ui.celebrate?.();
    } catch (err) {
      toast(`Couldn’t set it up — ${err}`, { kind: 'error' });
    } finally { repaintSettings(); }
  }
  async function claudeRemove(b) {
    const ok = await confirmDialog({
      title: 'Remove wridge from Claude Code?',
      body: 'Takes the wridge MCP server and wridge’s memory hook out of Claude Code’s settings. Nothing else changes. You can set it up again any time.',
      confirm: 'Remove', danger: true,
    });
    if (!ok) return;
    b.disabled = true;
    try {
      claude = await invoke('claude_remove');
      claudeAt = Date.now();
      toast('Removed — Claude Code won’t see wridge in new sessions');
    } catch (err) {
      toast(`Couldn’t remove it — ${err}`, { kind: 'error' });
    } finally { repaintSettings(); }
  }

  // ---------- data ----------
  let settingsSig = '';
  function update() {
    paintSidebar();
    if (route === 'overview') renderOverview();
    if (route === 'settings') {
      const sig = JSON.stringify([S?.running, S?.engine, S?.pluginBuilt, http?.pluginConnected, http?.fileName, kind(), http?.version, figFiles().map((f) => f.pluginVersion)]);
      if (sig !== settingsSig) { settingsSig = sig; const y = views.settings.scrollTop; renderSettings(); views.settings.scrollTop = y; }
    }
    if (PAGES[route]) {
      const off = !$('#page-off').hidden;
      if (off && serverUp()) loadPage(route, true);
      else if (!serverUp()) loadPage(route, false);
    }
  }
  async function loadProjects() {
    try { projects = await api('/api/code/projects'); } catch { return; }
    if (!projects.some((p) => p.dir === project)) { project = projects[0]?.dir ?? ''; store.set('wridge-project', project); }
  }
  async function loadBrain() {
    if (!project || !httpOk) return;
    try {
      const g = await api(`/api/brain?dir=${encodeURIComponent(project)}`);
      brain = { notes: g.nodes.filter((n) => n.kind === 'note').length, links: g.edges.length, broken: g.nodes.filter((n) => n.kind === 'missing').length, trash: g.trash?.length ?? 0 };
    } catch { brain = null; }
    update();
  }
  let pollTimer = null, brainAt = 0;
  async function poll() {
    clearTimeout(pollTimer);
    const was = httpOk;
    const hadFigma = Boolean(http?.pluginConnected);
    if (S?.running || !T) {
      try { http = await api('/api/status'); httpOk = true; } catch { httpOk = false; }
      if (httpOk) { try { figma = await api('/api/sessions'); figmaChanged(); } catch { figma = null; } }
    } else { httpOk = false; http = null; figma = null; }
    if (!T) S = await invoke('server_status');
    if (httpOk && (!was || !projects.length)) await loadProjects();
    if (hadFigma === false && http?.pluginConnected && was) {
      toast(`Figma connected${shown()?.fileName ?? http.fileName ? ` · ${shown()?.fileName ?? http.fileName}` : ''}`);
      setTimeout(() => { const br = document.querySelector('.bridge'); if (br) { br.classList.add('just'); setTimeout(() => br.classList.remove('just'), 700); } }, 50);
    }
    if (httpOk && (!was || Date.now() - brainAt > 20000)) { brainAt = Date.now(); loadBrain(); }
    update();
    pollTimer = setTimeout(poll, kind() === 'starting' ? 700 : document.hidden ? 10000 : 2500);
  }

  // ---------- small motion on a timer (one-shots: nothing runs between them) ----------
  let blinkN = 0;
  setInterval(() => {
    if (document.hidden || root.classList.contains('resizing')) return;
    const eye = document.querySelector('.node .logo[data-logo="logo-wridge"]');
    if (eye) { const cls = ++blinkN % 3 === 0 ? 'blink2' : 'blink'; eye.classList.remove('blink', 'blink2'); void eye.offsetWidth; eye.classList.add(cls); setTimeout(() => eye.classList.remove(cls), 600); }
  }, 3000);
  setInterval(() => {
    if (document.hidden || root.classList.contains('resizing')) return;
    const fig = document.querySelector('.node:not(.off) .logo[data-logo="logo-figma"]');
    if (fig) { fig.classList.remove('pop'); void fig.offsetWidth; fig.classList.add('pop'); setTimeout(() => fig.classList.remove('pop'), 1300); }
  }, 3200);
  // pause decorative loops while the window edge is being dragged
  let resizeT = 0;
  window.addEventListener('resize', () => { root.classList.add('resizing'); clearTimeout(resizeT); resizeT = setTimeout(() => root.classList.remove('resizing'), 200); });

  // ---------- start ----------
  (async () => {
    S = await invoke('server_status');
    try { appVersion = (await T?.app?.getVersion?.()) ?? ''; } catch { appVersion = ''; }
    try { prefs = await invoke('get_prefs'); } catch { /* defaults */ }
    try { logs = await invoke('server_logs'); } catch { logs = []; }
    listen('log', (line) => { logs.push(line); if (logs.length > 600) logs.splice(0, logs.length - 600); renderLogs(); if (route === 'overview' && ['stopped', 'error'].includes(kind())) renderOverview(); });
    listen('status', (s) => { S = s; if (!s.running) { httpOk = false; http = null; } update(); poll(); });
    listen('navigate', (r) => { location.hash = `#/${r}`; });
    applyTheme(); // also gives the native window the theme's background
    $('.sb-logo').classList.add('mi-intro');
    setTimeout(() => $('.sb-logo').classList.remove('mi-intro'), 1600);
    show(fromHash(), { force: true });
    poll();
    document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });
  })();
})();
