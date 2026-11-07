// Shared 3D graph setup for the design graph (/graph) and the code graph (/code):
// smooth by default (plain lines, layout settled before the first frame, capped pixel ratio, no drawing
// while hidden or idle), prettier (depth fog, floating labels for the important nodes, light/dark theme),
// and particles that run only along the selected node's links.
window.Graph3D = (() => {
  const THEMES = {
    light: { bg: 'rgba(0,0,0,0)', fog: '#EEF3FA', link: 'rgba(100,116,139,.32)', linkDim: 'rgba(148,163,184,.07)', nodeDim: 'rgba(148,163,184,.22)', nodeFaint: 'rgba(148,163,184,.07)', particle: '#2563EB' },
    dark: { bg: 'rgba(0,0,0,0)', fog: '#0B1124', link: 'rgba(148,163,184,.30)', linkDim: 'rgba(100,116,139,.08)', nodeDim: 'rgba(71,85,105,.35)', nodeFaint: 'rgba(71,85,105,.12)', particle: '#93C5FD' },
  };
  const KEY = 'wara-graph3d-theme';
  const MAX_PARTICLE_LINKS = 30;
  const IDLE_MS = 2500;

  let themeName = 'light';
  try { themeName = localStorage.getItem(KEY) === 'dark' ? 'dark' : 'light'; } catch { /* storage blocked: light */ }

  // three.js for the few objects the bundled graph library does not expose (fog). Loaded once, lazily.
  let threeReady = null;
  const three = () => (threeReady ??= import('/vendor/three.module.js').catch(() => null));

  /**
   * opts: { important(n) → number (higher = label first), label(n) → text, labels = 12,
   *         particleColor?, onTheme?(name) }
   * Returns the ForceGraph3D instance with extra methods: setActive(links), setTheme(name), theme().
   */
  function create(el, opts = {}) {
    const g = ForceGraph3D({ controlType: 'orbit', rendererConfig: { antialias: true, powerPreference: 'high-performance' } })(el)
      .showNavInfo(false)
      .nodeResolution(10)
      .linkWidth(0) // plain lines: tubes per link were the main cause of stutter
      .warmupTicks(90) // lay out before the first frame, so the graph appears settled
      .cooldownTicks(80)
      .d3VelocityDecay(0.35)
      .linkDirectionalParticleWidth(1.8)
      .linkDirectionalParticleResolution(6)
      .linkDirectionalParticleSpeed(0.006);
    g.renderer().setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));

    // ---- particles only on the active (selected) links ----
    let active = new Set();
    g.linkDirectionalParticles((l) => (active.has(l) ? 2 : 0));
    g.setActive = (links) => {
      active = new Set((links ?? []).slice(0, MAX_PARTICLE_LINKS));
      focusing = (links ?? []).length > 0;
      if (fog) g.scene().fog = focusing ? null : fog; // fog would wash out far neighbours of the selection
      g.linkDirectionalParticles(g.linkDirectionalParticles()); // re-evaluate
      wake();
      return g;
    };

    // ---- theme ----
    let fog = null;
    let focusing = false;
    const applyTheme = () => {
      const t = THEMES[themeName];
      g.backgroundColor(t.bg).linkDirectionalParticleColor(() => opts.particleColor ?? t.particle);
      document.body.classList.toggle('graph3d-dark', themeName === 'dark');
      three().then((THREE) => {
        if (!THREE) return;
        fog ??= new THREE.Fog(t.fog, 200, 2000);
        fog.color.set(t.fog);
        g.scene().fog = focusing ? null : fog;
        fitFog();
      });
      opts.onTheme?.(themeName);
    };
    // Fog measured from the graph itself: the near half stays crisp, the far side fades. Uses the
    // camera's distance to the graph's centre, so zooming in or out (also tweened) keeps it right.
    let bounds = { cx: 0, cy: 0, cz: 0, r: 300 };
    const measure = () => {
      const b = g.getGraphBbox?.();
      if (!b) return;
      bounds = {
        cx: (b.x[0] + b.x[1]) / 2, cy: (b.y[0] + b.y[1]) / 2, cz: (b.z[0] + b.z[1]) / 2,
        r: Math.max(60, Math.hypot(b.x[1] - b.x[0], b.y[1] - b.y[0], b.z[1] - b.z[0]) / 2),
      };
    };
    const fitFog = () => {
      if (!fog) return;
      const p = g.camera().position;
      const d = Math.hypot(p.x - bounds.cx, p.y - bounds.cy, p.z - bounds.cz);
      fog.near = Math.max(1, d - bounds.r * 0.2);
      fog.far = d + bounds.r * 1.6;
    };
    g.setTheme = (name) => {
      themeName = name === 'dark' ? 'dark' : 'light';
      try { localStorage.setItem(KEY, themeName); } catch { /* not persisted */ }
      applyTheme();
      wake();
      return g;
    };
    g.theme = () => themeName;
    g.palette = () => THEMES[themeName];

    // ---- floating labels for the most important nodes (HTML: crisp, and far cheaper than 3D text) ----
    const layer = document.createElement('div');
    layer.className = 'g3d-labels';
    el.appendChild(layer);
    const pool = [];
    let labelled = [];
    const pickLabels = () => {
      const { nodes } = g.graphData();
      const max = opts.labels ?? 12;
      labelled = opts.important ? nodes.filter((n) => opts.important(n) > 0).sort((a, b) => opts.important(b) - opts.important(a)).slice(0, max) : [];
      while (pool.length < labelled.length) {
        const d = document.createElement('div');
        d.className = 'g3d-label';
        layer.appendChild(d);
        pool.push(d);
      }
      pool.forEach((d, i) => {
        d.style.display = i < labelled.length ? '' : 'none';
        if (i >= labelled.length) return;
        d.textContent = (opts.label ?? ((n) => n.name ?? n.id))(labelled[i]);
        d.className = `g3d-label ${opts.labelClass?.(labelled[i]) ?? ''}`.trim(); // e.g. the current search hit
      });
      // Sizes measured once per text change (reading them every frame would force layout each frame).
      sizes = labelled.map((_, i) => [pool[i].offsetWidth, pool[i].offsetHeight]);
    };
    let sizes = [];
    const placeLabels = () => {
      const cam = g.camera();
      const m = cam.matrixWorld.elements; // camera looks along -Z of its world matrix
      const dir = [-m[8], -m[9], -m[10]];
      // Most important first; a label that would overlap one already placed is hidden for this frame.
      const placed = [];
      labelled.forEach((n, i) => {
        const d = pool[i];
        const ahead = (n.x - cam.position.x) * dir[0] + (n.y - cam.position.y) * dir[1] + (n.z - cam.position.z) * dir[2];
        if (!(ahead > 0) || n.x === undefined) { d.style.opacity = '0'; return; }
        const { x, y } = g.graph2ScreenCoords(n.x, n.y, n.z);
        const [w, h] = sizes[i] ?? [80, 20];
        const box = [x - w / 2, y - h * 1.5, x + w / 2, y - h * 0.5];
        if (placed.some((b) => box[0] < b[2] + 4 && box[2] + 4 > b[0] && box[1] < b[3] + 2 && box[3] + 2 > b[1])) { d.style.opacity = '0'; return; }
        placed.push(box);
        d.style.opacity = '';
        d.style.transform = `translate(${x}px, ${y}px) translate(-50%, -150%)`;
      });
    };
    g.refreshLabels = () => { pickLabels(); placeLabels(); return g; };

    // ---- draw only when something moves: pause when hidden, or idle with no particles running ----
    let shown = false;
    let idleTimer = null;
    let paused = false;
    function wake() {
      if (!shown || document.hidden) return;
      if (paused) { g.resumeAnimation(); paused = false; }
      startFollow();
      clearTimeout(idleTimer);
      idleTimer = setTimeout(sleep, IDLE_MS);
    }
    function sleep() {
      if (!shown || active.size || engineRunning || rotating) { idleTimer = setTimeout(sleep, IDLE_MS); return; }
      g.pauseAnimation();
      paused = true;
    }
    let engineRunning = true;
    let pageOnStop = null;
    g.onEngineTick(() => { engineRunning = true; });
    g.onEngineStop(() => { engineRunning = false; measure(); pageOnStop?.(); });
    // Pages set their own onEngineStop (zoom to fit): chain it instead of replacing ours.
    g.onEngineStop = (cb) => { pageOnStop = cb; return g; };
    // Labels and fog follow the camera every frame it moves — mouse, wheel or a tweened move — and the
    // loop only runs while the graph is drawing (it stops with the render loop when idle or hidden).
    let lastCam = '';
    const follow = () => {
      if (paused || !shown) { following = false; return; }
      const c = g.camera().position;
      const key = `${c.x.toFixed(1)},${c.y.toFixed(1)},${c.z.toFixed(1)}|${engineRunning}`;
      if (key !== lastCam || engineRunning) { lastCam = key; placeLabels(); fitFog(); }
      requestAnimationFrame(follow);
    };
    let following = false;
    const startFollow = () => { if (!following) { following = true; requestAnimationFrame(follow); } };
    for (const type of ['pointerdown', 'pointermove', 'wheel', 'keydown']) el.addEventListener(type, wake, { passive: true });
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) { g.pauseAnimation(); paused = true; } else wake();
    });
    // Camera moves are tweened inside the render loop, so they wake it first. While the page is hidden
    // the loop cannot run, so the move happens at once instead of being lost (it would never finish).
    const TWEEN_ARG = { cameraPosition: 2, zoomToFit: 0 };
    for (const name of Object.keys(TWEEN_ARG)) {
      const original = g[name].bind(g);
      g[name] = (...args) => {
        if (!args.length) return original();
        if (document.hidden) args[TWEEN_ARG[name]] = 0;
        else wake();
        return original(...args);
      };
    }
    const graphData = g.graphData.bind(g);
    g.graphData = (...args) => {
      if (!args.length) return graphData();
      engineRunning = true;
      graphData(...args);
      pickLabels();
      wake();
      return g;
    };
    // Preview: slow orbit from wherever the camera is now (no re-fit), around what it looks at;
    // keeps drawing while on.
    let rotating = false;
    g.setRotating = (on) => {
      rotating = Boolean(on);
      const c = g.controls();
      c.autoRotate = rotating;
      c.autoRotateSpeed = 0.7;
      wake();
      return g;
    };
    g.rotating = () => rotating;
    g.show = () => { shown = true; layer.style.display = ''; wake(); return g; };
    g.hide = () => { shown = false; layer.style.display = 'none'; clearTimeout(idleTimer); g.pauseAnimation(); paused = true; return g; };

    applyTheme();
    return g;
  }

  // The light/dark switch button shared by both pages.
  function themeButton(btn, g) {
    const paint = () => {
      const dark = g.theme() === 'dark';
      btn.textContent = dark ? 'Light theme' : 'Dark theme';
      btn.setAttribute('aria-pressed', String(dark));
    };
    btn.onclick = () => { g.setTheme(g.theme() === 'dark' ? 'light' : 'dark'); paint(); };
    paint();
  }

  // The "หมุนดูรอบ" (orbit preview) toggle shared by both pages.
  function rotateButton(btn, g) {
    const paint = () => {
      btn.textContent = g.rotating() ? 'Stop orbit' : 'Orbit';
      btn.setAttribute('aria-pressed', String(g.rotating()));
    };
    btn.onclick = () => { g.setRotating(!g.rotating()); paint(); };
    paint();
  }

  return { create, themeButton, rotateButton, themes: THEMES };
})();
