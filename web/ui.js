// wridge UI kit — shared helpers for every page (toasts, dialogs, icons, Lottie art).
(() => {
  // ---- Inside the wridge window: a page sits in an iframe next to the app's own sidebar. It then skips
  // its side menu, takes the window's theme, and tells the window where it is (for the active nav item).
  const EMBED = window.top !== window;
  const setTheme = (t) => { if (t === 'dark' || t === 'light') document.documentElement.dataset.theme = t; };
  if (EMBED) {
    document.documentElement.classList.add('embed');
    setTheme(new URLSearchParams(location.search).get('theme'));
    window.addEventListener('message', (e) => { if (e.data?.type === 'wridge:theme') setTheme(e.data.theme); });
    const tell = () => window.parent.postMessage({ type: 'wridge:path', path: location.pathname, search: location.search, title: document.title }, '*');
    for (const fn of ['pushState', 'replaceState']) {
      const orig = history[fn].bind(history);
      history[fn] = (...args) => { const r = orig(...args); tell(); return r; };
    }
    window.addEventListener('popstate', tell);
    window.addEventListener('DOMContentLoaded', tell);
  }

  // ---- The wridge window and the pages inside it are one session ("app"): the Figma file picked in the
  // sidebar is the one they read, and that pick never moves an AI's file. A page opened in a browser stays
  // in the shared session, as before.
  const APP_WINDOW = EMBED || /tauri/.test(location.origin) || location.pathname === '/app';
  if (APP_WINDOW) {
    const send = window.fetch.bind(window);
    window.fetch = (input, init = {}) => {
      const url = new URL(typeof input === 'string' ? input : input.url, location.href);
      if (!/^\/(api|mcp)\b/.test(url.pathname) || !/^(127\.0\.0\.1|localhost):\d+$/.test(url.host)) return send(input, init);
      const headers = new Headers(init.headers ?? (typeof input === 'string' ? undefined : input.headers));
      if (!headers.has('X-Wridge-Session')) headers.set('X-Wridge-Session', 'app');
      return send(input, { ...init, headers });
    };
  }

  const ICONS = {
    logo: '<svg viewBox="0 0 32 21" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true"><path d="M0.768066 8.32021L2.04807 0.512207L8.32007 5.37621L0.768066 8.32021Z" fill="#111111"/><path d="M31.2322 8.32021L29.9522 0.512207L23.6802 5.37621L31.2322 8.32021Z" fill="#2F6BFF"/><path d="M9.60014 5.12012H4.48014C2.35936 5.12012 0.640137 6.83934 0.640137 8.96012V16.6401C0.640137 18.7609 2.35936 20.4801 4.48014 20.4801H9.60014C11.7209 20.4801 13.4401 18.7609 13.4401 16.6401V8.96012C13.4401 6.83934 11.7209 5.12012 9.60014 5.12012Z" fill="#111111"/><path d="M27.5201 5.12012H22.4001C20.2793 5.12012 18.5601 6.83934 18.5601 8.96012V16.6401C18.5601 18.7609 20.2793 20.4801 22.4001 20.4801H27.5201C29.6408 20.4801 31.3601 18.7609 31.3601 16.6401V8.96012C31.3601 6.83934 29.6408 5.12012 27.5201 5.12012Z" fill="#2F6BFF"/><path d="M19.1999 10.8799H12.7999C11.7395 10.8799 10.8799 11.7395 10.8799 12.7999C10.8799 13.8603 11.7395 14.7199 12.7999 14.7199H19.1999C20.2603 14.7199 21.1199 13.8603 21.1199 12.7999C21.1199 11.7395 20.2603 10.8799 19.1999 10.8799Z" fill="#111111"/><path d="M7.29619 14.5919C8.56865 14.5919 9.60019 13.5604 9.60019 12.2879C9.60019 11.0154 8.56865 9.98389 7.29619 9.98389C6.02372 9.98389 4.99219 11.0154 4.99219 12.2879C4.99219 13.5604 6.02372 14.5919 7.29619 14.5919Z" fill="#FBFAF8"/><path d="M24.7039 14.5919C25.9764 14.5919 27.0079 13.5604 27.0079 12.2879C27.0079 11.0154 25.9764 9.98389 24.7039 9.98389C23.4314 9.98389 22.3999 11.0154 22.3999 12.2879C22.3999 13.5604 23.4314 14.5919 24.7039 14.5919Z" fill="#FBFAF8"/></svg>',
    search: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>',
    check: '<svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="#22C55E"/><path d="m6 10.2 2.6 2.6L14 7.4" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
    cross: '<svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="#EF4444"/><path d="m7 7 6 6m0-6-6 6" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>',
    info: '<svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><circle cx="10" cy="10" r="9" fill="currentColor"/><path d="M10 9v5" stroke="#fff" stroke-width="2" stroke-linecap="round"/><circle cx="10" cy="6.2" r="1.2" fill="#fff"/></svg>',
    warn: '<svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="M8.3 2.9a2 2 0 0 1 3.4 0l6.6 11.4A2 2 0 0 1 16.6 17H3.4a2 2 0 0 1-1.7-2.7Z" fill="currentColor"/><path d="M10 7.5v4" stroke="#fff" stroke-width="2" stroke-linecap="round"/><circle cx="10" cy="14" r="1.1" fill="#fff"/></svg>',
    close: '<svg viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="m4 4 8 8m0-8-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>',
    inbox: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 13h5l1.5 2.5h5L16 13h5"/><path d="M5.5 5h13L21 13v5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18v-5Z"/></svg>',
    alert: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10.3 3.9a2 2 0 0 1 3.4 0l7.5 13A2 2 0 0 1 19.5 20h-15a2 2 0 0 1-1.7-3.1Z"/><path d="M12 9.5v4"/><path d="M12 16.8h.01"/></svg>',
    trash: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16"/><path d="M10 11v6M14 11v6"/><path d="M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12"/><path d="M9 7V4h6v3"/></svg>',
    unplug: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 2v5M15 2v5"/><path d="M6 7h12v4a6 6 0 0 1-12 0Z"/><path d="M12 17v5"/><path d="m3 3 18 18"/></svg>',
  };

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  // Fill every [data-icon] placeholder (keeps the HTML readable).
  function icons(root = document) {
    root.querySelectorAll('[data-icon]').forEach((el) => { el.innerHTML = ICONS[el.dataset.icon] ?? ''; });
  }

  function chip(el, kind, text) {
    el.className = `chip ${kind}`;
    el.textContent = text;
  }

  // Toast: success disappears by itself; errors stay until closed.
  function toast(message, { kind = 'success', action, onAction, duration } = {}) {
    let box = document.querySelector('.toasts');
    if (!box) {
      box = document.createElement('div');
      box.className = 'toasts';
      box.setAttribute('role', 'status');
      box.setAttribute('aria-live', 'polite');
      document.body.append(box);
    }
    const t = document.createElement('div');
    t.className = 'toast';
    t.innerHTML = `${kind === 'error' ? ICONS.cross : kind === 'info' ? ICONS.info.replace('currentColor', '#3B82F6') : ICONS.check}
      <span>${esc(message)}</span>${action ? `<button class="toast-action">${esc(action)}</button>` : ''}
      <button class="toast-close" aria-label="Close">${ICONS.close}</button>`;
    const close = () => { t.classList.add('leaving'); setTimeout(() => t.remove(), 180); };
    t.querySelector('.toast-close').onclick = close;
    if (action) t.querySelector('.toast-action').onclick = () => { close(); onAction?.(); };
    box.append(t);
    const ms = duration ?? (kind === 'error' ? 0 : 2500);
    if (ms) setTimeout(close, ms);
    return close;
  }

  // Confirm dialog → Promise<boolean>. Cancel, Escape and clicking the backdrop all resolve false.
  function confirmDialog({ title, body, confirm, danger = false }) {
    return new Promise((resolve) => {
      const d = document.createElement('dialog');
      d.className = 'dialog';
      d.innerHTML = `<form method="dialog" class="dialog-body">
        ${danger ? `<div class="dialog-icon">${ICONS.warn}</div>` : ''}
        <div><h2>${esc(title)}</h2><p>${esc(body)}</p></div>
        <div class="dialog-actions">
          <button class="btn btn-secondary" value="cancel">Cancel</button>
          <button class="btn ${danger ? 'btn-danger-solid' : 'btn-primary'}" value="ok">${esc(confirm)}</button>
        </div></form>`;
      // Answer on the button itself (submit), not only on "close": that event can lag in a background tab.
      const done = (ok) => { resolve(ok); d.remove(); };
      d.querySelector('form').addEventListener('submit', (e) => { e.preventDefault(); done(e.submitter?.value === 'ok'); });
      d.addEventListener('click', (e) => { if (e.target === d) done(false); });
      d.addEventListener('cancel', (e) => { e.preventDefault(); done(false); }); // Esc
      document.body.append(d);
      d.showModal();
      d.querySelector('[value="cancel"]').focus();
    });
  }

  // Dialog with one text field → Promise<string | null>.
  function inputDialog({ title, body, label, placeholder = '', value = '', confirm, note }) {
    return new Promise((resolve) => {
      const d = document.createElement('dialog');
      d.className = 'dialog';
      d.innerHTML = `<form method="dialog" class="dialog-body">
        <div><h2>${esc(title)}</h2>${body ? `<p>${esc(body)}</p>` : ''}</div>
        <label class="field"><span>${esc(label)}</span><input class="field-input" name="value" placeholder="${esc(placeholder)}" value="${esc(value)}" required autocomplete="off" spellcheck="false"></label>
        ${note ? `<p class="field-note">${esc(note)}</p>` : ''}
        <div class="dialog-actions">
          <button class="btn btn-secondary" type="button" data-cancel>Cancel</button>
          <button class="btn btn-primary" value="ok">${esc(confirm)}</button>
        </div></form>`;
      // Cancel is not a submit button, so Enter in the field confirms instead of cancelling.
      const input = d.querySelector('input');
      const done = (value) => { resolve(value); d.remove(); };
      d.querySelector('[data-cancel]').onclick = () => done(null);
      d.querySelector('form').addEventListener('submit', (e) => { e.preventDefault(); done(input.value.trim() || null); });
      d.addEventListener('click', (e) => { if (e.target === d) done(null); });
      d.addEventListener('cancel', (e) => { e.preventDefault(); done(null); }); // Esc
      document.body.append(d);
      d.showModal();
      input.focus();
    });
  }

  function setLoading(btn, loading, label) {
    if (loading) {
      btn.dataset.label ??= btn.textContent;
      btn.classList.add('is-loading');
      btn.innerHTML = `<span class="spinner"></span>${esc(label ?? btn.dataset.label)}`;
    } else {
      btn.classList.remove('is-loading');
      btn.textContent = btn.dataset.label ?? btn.textContent;
      delete btn.dataset.label;
    }
  }

  // ---- Lottie art (server/lottie/*.json, approved in Figma "Lottie art v2") ----
  // Any element with data-anim="<name>" plays it (data-loop to repeat); added later too (observer).
  // Reduced motion: the poster frame, no movement. The player library loads only when a page uses one.
  const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
  let lottieLib = null;
  const loadLottie = () => (lottieLib ??= window.lottie ? Promise.resolve(window.lottie) : new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = '/vendor/lottie.min.js';
    s.onload = () => resolve(window.lottie);
    s.onerror = reject;
    document.head.append(s);
  }));
  const animData = new Map();
  const fetchAnim = (name) => {
    if (!animData.has(name)) animData.set(name, fetch(`/lottie/${encodeURIComponent(name)}.json`).then((r) => (r.ok ? r.json() : Promise.reject(new Error(`no ${name}`)))));
    return animData.get(name);
  };
  const posterOf = (data) => { const ms = data.meta?.wara?.spec?.poster; const last = data.op - (data.ip ?? 0) - 1; return ms == null ? last : Math.min(last, Math.round((ms / 1000) * data.fr)); };
  async function playAnim(el, name, { loop = false, onDone } = {}) {
    try {
      const [lib, data] = await Promise.all([loadLottie(), fetchAnim(name)]);
      if (!el.isConnected) return null;
      el.innerHTML = '';
      const a = lib.loadAnimation({ container: el, renderer: 'svg', loop: loop && !reducedMotion(), autoplay: false, animationData: JSON.parse(JSON.stringify(data)) });
      a.addEventListener('DOMLoaded', () => { if (reducedMotion()) { a.goToAndStop(posterOf(data), true); onDone?.(); } else a.play(); });
      if (onDone) a.addEventListener('complete', onDone);
      el._anim?.destroy();
      el._anim = a;
      return a;
    } catch { return null; } // art missing: the page keeps working without it
  }
  const mountAll = (root) => root.querySelectorAll?.('[data-anim]:not([data-anim-on])').forEach((el) => {
    el.setAttribute('data-anim-on', '');
    playAnim(el, el.dataset.anim, { loop: el.hasAttribute('data-loop') });
  });
  new MutationObserver((list) => { for (const m of list) for (const n of m.addedNodes) if (n.nodeType === 1) { if (n.matches('[data-anim]')) mountAll(n.parentNode); else mountAll(n); } })
    .observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener('DOMContentLoaded', () => mountAll(document));

  // Empty / error / waiting states: the user's sticker, floating (Figma "wridge Motion · Micro" → Sticker float).
  // A mood can be named; otherwise it follows the icon: inbox → thinking, alert → stressed, unplug → phone.
  const STICKER_FOR = { inbox: 'thinking', alert: 'stressed', unplug: 'phone', search: 'thinking' };
  const emptyIcon = (icon, kind = '', mood) => `<div class="empty-sticker${kind ? ` ${kind}` : ''}" aria-hidden="true"><img src="/stickers/${mood ?? STICKER_FOR[icon] ?? 'thinking'}.png" alt="" draggable="false"></div>`;

  // A short "done!" burst above the toasts (saved, sent). Skipped under reduced motion.
  function celebrate() {
    if (reducedMotion()) return;
    const el = document.createElement('div');
    el.className = 'celebrate';
    el.setAttribute('aria-hidden', 'true');
    document.body.append(el);
    playAnim(el, 'success', { onDone: () => { el.classList.add('leaving'); setTimeout(() => el.remove(), 250); } });
    setTimeout(() => el.remove(), 4000);
  }

  // ---- Project picker: choose, search, add (builds the code graph) and remove projects ----
  // host: an empty element in the top bar. opts: projects() → [{dir, exists}], current() → dir,
  // onChoose(dir) — switch to it; onChanged(dir | null) — the list changed (dir = the one to open);
  // canLeave?() → false to stay (unsaved work).
  const baseName = (p) => String(p).replaceAll('\\', '/').split('/').pop();
  function projectPicker(host, opts) {
    host.classList.add('picker');
    const CHEVRON = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 6 4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>';
    host.innerHTML = `<button class="btn btn-secondary picker-btn" aria-haspopup="listbox" aria-expanded="false"><span>Pick a project</span>${CHEVRON}</button>
      <div class="picker-pop" hidden>
        <label class="input">${ICONS.search}<input type="search" placeholder="Find a project…" aria-label="Find a project" autocomplete="off"></label>
        <div class="picker-list" role="listbox" aria-label="Projects"></div>
        <button class="picker-add">+ Add a project</button>
      </div>`;
    const btn = host.querySelector('.picker-btn'), pop = host.querySelector('.picker-pop'), input = pop.querySelector('input'), list = pop.querySelector('.picker-list');
    let active = 0, shown = [];
    const isOpen = () => !pop.hidden;
    function label() {
      const cur = opts.current();
      btn.innerHTML = `<span>${esc(cur ? baseName(cur) : 'Pick a project')}</span>${CHEVRON}`; // rebuilt: setLoading leaves plain text
      delete btn.dataset.label;
      host.hidden = false;
    }
    function draw() {
      const q = input.value.trim().toLowerCase(), cur = opts.current();
      shown = opts.projects().filter((p) => !q || p.dir.toLowerCase().includes(q));
      active = Math.min(active, Math.max(0, shown.length - 1));
      list.innerHTML = shown.length ? shown.map((p, i) => `
        <div class="picker-row${p.dir === cur ? ' current' : ''}${i === active ? ' active' : ''}" role="option" aria-selected="${p.dir === cur}">
          <button class="picker-item" data-pick="${i}"><span class="picker-name">${esc(baseName(p.dir))}</span>
            <span class="picker-path${p.exists === false ? ' picker-missing' : ''}">${p.exists === false ? 'folder missing · ' : ''}${esc(p.dir)}</span></button>
          <button class="picker-del" data-del="${i}" aria-label="Remove ${esc(baseName(p.dir))} from the list" title="Remove from the list">${ICONS.trash}</button>
        </div>`).join('') : `<div class="picker-empty">${opts.projects().length ? `No project matches “${esc(input.value)}”` : 'No projects yet'}</div>`;
      list.querySelectorAll('[data-pick]').forEach((b) => (b.onclick = () => choose(shown[Number(b.dataset.pick)])));
      list.querySelectorAll('[data-del]').forEach((b) => (b.onclick = () => { const p = shown[Number(b.dataset.del)]; close(); remove(p.dir); }));
    }
    function open() {
      pop.hidden = false;
      btn.setAttribute('aria-expanded', 'true');
      input.value = '';
      active = Math.max(0, opts.projects().findIndex((p) => p.dir === opts.current()));
      draw();
      input.focus();
    }
    function close() { pop.hidden = true; btn.setAttribute('aria-expanded', 'false'); }
    function choose(p) {
      close();
      if (!p || p.dir === opts.current() || opts.canLeave?.() === false) return;
      opts.onChoose(p.dir);
      label();
    }
    async function add() {
      close();
      if (opts.canLeave?.() === false) return;
      const dir = await inputDialog({
        title: 'Add a project',
        body: 'wridge reads the code and keeps its graph in the project’s .wara/ folder. Your code is never changed.',
        label: 'Project folder path',
        placeholder: 'C:\\Users\\you\\Projects\\my-app',
        note: 'In a git repo, /.wara/ is added to .gitignore first.',
        confirm: 'Build the graph',
      });
      if (!dir) return;
      setLoading(btn, true, 'Building the graph…');
      try {
        const res = await fetch('/api/code/projects', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ dir: dir.replace(/^"|"$/g, '') }) });
        const body = await res.json();
        if (!res.ok) throw new Error(body.error);
        toast(`Graph built · ${body.files} files${body.gitignore?.added ? ' · .wara/ added to .gitignore' : ''}`);
        celebrate();
        setLoading(btn, false);
        await opts.onChanged(body.root);
      } catch (err) {
        setLoading(btn, false);
        toast(`Couldn’t add the project — ${err.message}`, { kind: 'error' });
      }
      label();
    }
    // Only leaves this list: the code and its .wara/ folder stay, so adding it back is instant.
    async function remove(dir) {
      const ok = await confirmDialog({ title: `Remove ${baseName(dir)} from the list?`, body: 'Only the list entry goes — the code and the project’s .wara/ folder stay. You can add it back any time.', confirm: 'Remove', danger: true });
      if (!ok) return;
      try {
        const res = await fetch('/api/code/projects/remove', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ dir }) });
        if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error ?? `HTTP ${res.status}`);
      } catch (err) { return toast(`Couldn’t remove it — ${err.message}`, { kind: 'error' }); }
      toast(`Removed ${baseName(dir)} from the list`);
      await opts.onChanged(dir === opts.current() ? null : opts.current());
      label();
    }
    btn.onclick = () => (isOpen() ? close() : open());
    input.oninput = () => { active = 0; draw(); };
    input.onkeydown = (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); active = Math.min(active + 1, shown.length - 1); draw(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); active = Math.max(active - 1, 0); draw(); }
      else if (e.key === 'Enter') { e.preventDefault(); choose(shown[active]); }
      else if (e.key === 'Escape') { e.stopPropagation(); close(); btn.focus(); }
    };
    pop.querySelector('.picker-add').onclick = add;
    document.addEventListener('pointerdown', (e) => { if (isOpen() && !host.contains(e.target)) close(); });
    return { label, close, add, remove };
  }

  // ---- Side menu shared by every page (Figma "Brain · Prototype"): brain / design / code groups, the
  // project picker, and a status light. Each page's own top bar stays as its toolbar; its old nav is hidden.
  const NAV = [
    { label: 'Project', items: [['/tasks', 'Tasks'], ['/brain', 'Notes'], ['/reports', 'Reports'], ['/github', 'GitHub']] },
    { label: 'Figma', items: [['/graph', 'Design map'], ['/motion', 'Motion'], ['/pdf', 'PDF']] },
    { label: 'Code', code: true, items: [['/code', 'Code graph'], ['/data', 'Data'], ['/flows', 'Flows']] },
  ];
  const projectStore = { get: () => { try { return localStorage.getItem('code-project'); } catch { return null; } }, set: (v) => { try { localStorage.setItem('code-project', v); } catch { /* storage blocked */ } } };
  const here = () => {
    const view = new URLSearchParams(location.search).get('view');
    return location.pathname === '/brain' && view && view !== 'notes' ? `/brain?view=${view}` : location.pathname;
  };
  const item = ([href, label]) => `<a class="side-item" href="${href}" data-href="${href}"><span>${esc(label)}</span><span class="side-count" hidden></span></a>`;
  function shell() {
    if (document.querySelector('.shell') || document.body.hasAttribute('data-no-shell')) return; // the app's own control window has no side menu
    if (EMBED) return; // the wridge window draws the menu
    const side = document.createElement('aside');
    side.className = 'side-nav';
    side.setAttribute('aria-label', 'Menu');
    side.innerHTML = `<a class="wordmark" href="/brain"><span data-icon="logo"></span>wridge</a>
      <div class="side-project"></div>
      <nav class="side-groups">${NAV.map((g) => `<div class="side-group"${g.code ? ' data-code' : ''}><div class="side-label">${esc(g.label)}</div>${g.items.map(item).join('')}</div>`).join('')}</nav>
      <div class="side-foot">${item(['/skills', 'Skills'])}<a class="side-status" href="/app" title="Open the wridge window page"><span class="dot"></span><span class="txt">Checking…</span></a></div>`;
    const main = document.createElement('div');
    main.className = 'shell-main';
    main.append(...[...document.body.childNodes].filter((n) => !(n.nodeType === 1 && (n.matches('script') || n.matches('.toasts')))));
    const wrap = document.createElement('div');
    wrap.className = 'shell';
    wrap.append(side, main);
    document.body.prepend(wrap);
    // narrow screens: the side menu becomes a drawer behind this button
    const menuBtn = document.createElement('button');
    menuBtn.className = 'btn btn-ghost side-toggle';
    menuBtn.setAttribute('aria-label', 'Menu');
    menuBtn.textContent = '☰';
    menuBtn.onclick = (e) => { e.stopPropagation(); wrap.classList.toggle('nav-open'); };
    (main.querySelector('.topbar-left') ?? main).prepend(menuBtn);
    main.addEventListener('click', () => wrap.classList.remove('nav-open'));
    // the page's own project picker moves into the menu; pages without one get a shared one
    const slot = side.querySelector('.side-project');
    const own = main.querySelector('#picker');
    if (own) slot.append(own);
    shellProjects(slot, !own);
    setActive(here());
    shellStatus();
    icons(side);
  }
  function setActive(href) {
    if (href.startsWith('/brain?view=')) href = '/brain'; // Graph and Trash are tabs of Notes
    document.querySelectorAll('.side-item').forEach((a) => {
      if (a.dataset.href === href) a.setAttribute('aria-current', 'page');
      else a.removeAttribute('aria-current');
    });
  }
  function setCount(href, n) {
    const el = document.querySelector(`.side-item[data-href="${href}"] .side-count`);
    if (el) { el.textContent = n ?? ''; el.hidden = n === null || n === undefined; }
  }
  // Hides the code group for a project with no code the graph reads (a book, a game) — unless you are on a code page.
  async function shellProjects(slot, makePicker) {
    let projects = [];
    try { projects = await (await fetch('/api/code/projects', { cache: 'no-store' })).json(); } catch { return; }
    const current = () => new URLSearchParams(location.search).get('dir') ?? projectStore.get();
    const update = () => {
      const p = projects.find((x) => x.dir === current());
      const onCode = NAV[2].items.some(([h]) => h === location.pathname);
      document.querySelector('.side-group[data-code]')?.toggleAttribute('hidden', Boolean(p && !p.codeFiles && !onCode));
    };
    update();
    window.addEventListener('wara:project', update); // pages fire it after switching project
    if (!makePicker) return;
    const host = document.createElement('div');
    slot.append(host);
    projectPicker(host, {
      projects: () => projects, current,
      onChoose: (dir) => { projectStore.set(dir); const u = new URL(location.href); u.searchParams.delete('dir'); location.href = u; },
      onChanged: (dir) => { if (dir) projectStore.set(dir); location.reload(); },
    }).label();
  }
  async function shellStatus() {
    const el = document.querySelector('.side-status');
    const show = (kind, text) => { el.dataset.kind = kind; el.querySelector('.txt').textContent = text; };
    const tick = async () => {
      try {
        const s = await (await fetch('/api/status', { cache: 'no-store' })).json();
        if (s.paused) show('warn', 'Paused');
        else if (s.pluginConnected) show('ok', 'Ready · Figma connected');
        else show('warn', 'Waiting for Figma');
      } catch { show('off', 'Server stopped'); }
    };
    tick();
    setInterval(tick, 15000);
  }

  // Save a file the page made. In the wridge window, Windows' Save dialog asks where it goes (the window
  // does it) → { name, path }, or null when cancelled. In a browser it's a normal download → { name }.
  let saveN = 0;
  function saveFile(blob, name) {
    const plain = () => {
      const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: name });
      document.body.append(a); a.click(); a.remove();
      setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
      return { name };
    };
    if (!EMBED) return Promise.resolve(plain());
    const id = `save-${++saveN}`;
    return new Promise((resolve, reject) => {
      const on = (e) => {
        if (e.source !== window.parent || e.data?.type !== 'wridge:saved' || e.data.id !== id) return;
        window.removeEventListener('message', on);
        if (e.data.fallback) resolve(plain());
        else if (e.data.error) reject(new Error(e.data.error));
        else resolve(e.data.path ? { name: e.data.path.split(/[\\/]/).pop(), path: e.data.path } : null);
      };
      window.addEventListener('message', on);
      window.parent.postMessage({ type: 'wridge:save', id, name, blob }, '*');
    });
  }
  // open File Explorer at a saved file (inside the wridge window only)
  // copy text: the page's own clipboard, or (inside the wridge window, where the frame may not have it) the window's
  async function copy(text) {
    try { await navigator.clipboard.writeText(text); return true; } catch { if (!EMBED) return false; }
    const id = Math.random().toString(36).slice(2);
    return new Promise((done) => {
      const on = (e) => { if (e.source !== window.parent || e.data?.type !== 'wridge:copied' || e.data.id !== id) return; window.removeEventListener('message', on); done(Boolean(e.data.ok)); };
      window.addEventListener('message', on);
      window.parent.postMessage({ type: 'wridge:copy', id, text }, '*');
      setTimeout(() => { window.removeEventListener('message', on); done(false); }, 3000);
    });
  }
  const reveal = (path) => { if (EMBED && path) window.parent.postMessage({ type: 'wridge:reveal', path }, '*'); };

  // the Figma icon set (load /app-icons.js before ui.js): ui.icon('notes') → <svg>; wrap in <span class="w-icon">
  const icon = (name) => window.ICON?.[name] ?? '';
  // move the wridge window to one of its pages (overview, notes, graph, trash, design-map, motion, pdf, code,
  // data, flows, skills, settings); outside the window, open the matching web page
  const WEB = { overview: '/', notes: '/brain', graph: '/brain?view=graph', trash: '/brain?view=trash', 'design-map': '/graph', motion: '/motion', pdf: '/pdf', code: '/code', data: '/data', flows: '/flows', github: '/github', tasks: '/tasks', reports: '/reports', skills: '/skills', settings: '/' };
  const go = (route) => (EMBED ? window.parent.postMessage({ type: 'wridge:navigate', route }, '*') : (location.href = WEB[route] ?? '/'));
  window.ui = { ICONS, icon, go, esc, saveFile, reveal, copy, icons, chip, toast, confirmDialog, inputDialog, setLoading, playAnim, emptyIcon, celebrate, reducedMotion, projectPicker, nav: { setActive, setCount } };
  document.addEventListener('DOMContentLoaded', () => { shell(); icons(); });
})();
