// wridge.site — the bridge scrub, the Memory tabs, the companion stickers, the mark's eyes, the download link.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const root = document.documentElement;
  root.classList.add("js");
  const still = matchMedia("(prefers-reduced-motion: reduce)");
  const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));

  // ---- nav gets a hairline once the page moves
  const nav = $(".nav");
  const onScrollNav = () => nav.classList.toggle("scrolled", scrollY > 8);

  // ---- the mark's eyes look at the cursor (desktop pointers only)
  const eyes = $$(".nav .eye");
  if (matchMedia("(pointer: fine)").matches && !still.matches) {
    addEventListener("pointermove", (e) => {
      for (const eye of eyes) {
        const r = eye.getBoundingClientRect();
        const dx = e.clientX - (r.left + r.width / 2), dy = e.clientY - (r.top + r.height / 2);
        const d = Math.hypot(dx, dy) || 1, k = Math.min(1, d / 160) * 0.9;
        eye.style.transform = `translate(${(dx / d) * k}px, ${(dy / d) * k}px)`;
      }
    }, { passive: true });
  }

  // ---- stickers pop in when their section arrives (once)
  const io = new IntersectionObserver((entries) => {
    for (const en of entries) if (en.isIntersecting) { en.target.classList.add("in"); io.unobserve(en.target); }
  }, { threshold: 0.4 });
  $$(".pop").forEach((el) => io.observe(el));

  // ---- Memory and Plan: each list switches its own screenshot
  $$(".maps").forEach((box) => {
    const stageImg = box.querySelector(".map-stage img"), maps = [...box.querySelectorAll(".map")];
    maps.forEach((btn) => btn.addEventListener("click", () => {
      if (btn.getAttribute("aria-selected") === "true") return;
      maps.forEach((b) => b.setAttribute("aria-selected", String(b === btn)));
      const src = `assets/img/${btn.dataset.shot}.webp`;
      const next = new Image(); next.src = src;
      stageImg.classList.add("swap");
      const show = () => { stageImg.src = src; stageImg.alt = btn.dataset.alt; requestAnimationFrame(() => stageImg.classList.remove("swap")); };
      (next.decode ? next.decode().catch(() => {}) : Promise.resolve()).then(() => setTimeout(show, still.matches ? 0 : 160));
    }));
    // arrow keys move along the list
    box.querySelector(".map-list").addEventListener("keydown", (e) => {
      if (!["ArrowDown", "ArrowUp"].includes(e.key)) return;
      const i = maps.indexOf(document.activeElement);
      if (i < 0) return;
      e.preventDefault();
      const n = maps[(i + (e.key === "ArrowDown" ? 1 : maps.length - 1)) % maps.length];
      n.focus(); n.click();
    });
  });

  // ---- the bridge: scroll position drives the whole scene
  const run = $(".bridge-run"), track = $(".track"), frame = $(".frame"), size = $(".size");
  const cmds = $$(".cmds li"), steps = $$(".steps li"), caption = $(".caption");
  const NAMES = ["Overview", "Title", "Rows", "Back"];
  const COPY = ["You ask your AI for a screen, a fix, a flow.", "wridge carries each command across — one file at a time, in order.", "Figma builds it, and wridge remembers what you agreed."];
  let lastStep = -1;
  function paintBridge(p) {
    const work = clamp((p - 0.1) / 0.8); // 0 → 1 while the four commands cross
    const seg = Math.min(3, Math.floor(work * 4)), within = work * 4 - seg;
    cmds.forEach((li, i) => {
      li.classList.toggle("done", work >= 1 || i < seg);
      li.classList.toggle("flying", work > 0 && work < 1 && i === seg);
    });
    track.style.setProperty("--lit", work.toFixed(4));
    // the chip rides inside the track: its left edge goes from 12px to (track − chip − 12px)
    const chip = $(".in-flight", track), room = track.clientWidth - chip.offsetWidth - 24;
    chip.style.left = `${12 + Math.max(0, room) * within}px`;
    track.classList.toggle("idle", work <= 0 || work >= 1);
    track.classList.toggle("arrived", work >= 1);
    $(".in-flight b", track).textContent = cmds[seg].firstChild.nextSibling.textContent;
    // the screen builds top-down; the selection box hugs the newest band
    const reveal = work;
    const top = work >= 1 ? 0 : Math.max(0, reveal - 0.25), h = work >= 1 ? 1 : reveal - top;
    frame.style.setProperty("--reveal", reveal.toFixed(4));
    frame.style.setProperty("--sel-top", top.toFixed(4));
    frame.style.setProperty("--sel-h", h.toFixed(4));
    frame.style.setProperty("--sel-on", reveal > 0.02 ? 1 : 0);
    size.textContent = work >= 1 ? "Overview · 1280 × 820" : `${NAMES[seg]} · 1280 × ${Math.max(1, Math.round(h * 820))}`;
    // the three steps
    const step = p < 1 / 3 ? 0 : p < 2 / 3 ? 1 : 2;
    steps.forEach((li, i) => { li.classList.toggle("on", i <= step); li.querySelector(".bar").style.setProperty("--p", clamp(p * 3 - i).toFixed(3)); });
    if (step !== lastStep) { caption.textContent = COPY[step]; lastStep = step; }
  }
  // painted straight from the scroll event: a few style writes, no layout reads besides two rects
  function onScroll() {
    onScrollNav();
    if (!run || getComputedStyle(run).display === "none") return;
    if (still.matches) return paintBridge(1);
    const r = run.getBoundingClientRect();
    if (r.bottom < -200 || r.top > innerHeight + 200) return; // off screen
    paintBridge(clamp(-r.top / (r.height - innerHeight)));
  }
  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", onScroll);
  still.addEventListener?.("change", onScroll);
  onScroll();

  // ---- download buttons go straight to the newest installer (falls back to the Releases page)
  const KEY = "wridge-latest";
  const apply = (r) => {
    $$("[data-download]").forEach((a) => { a.href = r.url; });
    $$("[data-version]").forEach((s) => { s.textContent = r.version; });
    $$("[data-size]").forEach((s) => { s.textContent = r.size; });
  };
  try { const c = JSON.parse(sessionStorage.getItem(KEY)); if (c) apply(c); } catch { /* storage blocked */ }
  fetch("https://api.github.com/repos/Warayutkub/wridge/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((rel) => {
      const exe = rel.assets?.find((a) => /setup\.exe$/i.test(a.name));
      if (!exe) return;
      const r = { url: exe.browser_download_url, version: rel.tag_name.replace(/^v/, ""), size: `${Math.round(exe.size / 1048576)} MB` };
      apply(r);
      try { sessionStorage.setItem(KEY, JSON.stringify(r)); } catch { /* storage blocked */ }
    })
    .catch(() => { /* keep the Releases page link */ });
})();
