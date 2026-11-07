// wridge report renderer — draws a report's blocks (Figma "wridge Reports · Report blocks · Approved").
// Used by /reports (preview) and /report-view (the page Edge prints to PNG/PDF), so both look the same.
// Colors: default theme ← saved theme ← report.theme (merged by the server) ← block.style ← item.color.
(() => {
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  // Markdown, the small part: paragraphs, - lists, **bold**, *em*, `code`, [text](url)
  function md(text) {
    const inline = (s) => esc(s)
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>")
      .replace(/(^|[^*])\*([^*]+)\*/g, "$1<i>$2</i>")
      .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
    const out = []; let list = null;
    for (const raw of String(text ?? "").split(/\r?\n/)) {
      const line = raw.trimEnd();
      const li = line.match(/^\s*[-*•]\s+(.*)$/) || line.match(/^\s*\d+[.)]\s+(.*)$/);
      if (li) { (list ??= []).push(`<li>${inline(li[1])}</li>`); continue; }
      if (list) { out.push(`<ul>${list.join("")}</ul>`); list = null; }
      if (line.trim()) out.push(`<p>${inline(line)}</p>`);
    }
    if (list) out.push(`<ul>${list.join("")}</ul>`);
    return out.join("");
  }

  const CSS = `
  .wr { --page:#FBFAF8; font-family: var(--font, "IBM Plex Sans Thai"), "Segoe UI", system-ui, sans-serif; color: var(--text); background: var(--page); padding: 64px; display: flex; flex-direction: column; gap: 40px; box-sizing: border-box; }
  .wr * { box-sizing: border-box; }
  .wr p { margin: 0; }
  .wr .mono { font-family: var(--mono, "IBM Plex Mono"), ui-monospace, Consolas, monospace; }
  .wr .label { margin: 0; font: 500 11px/1.4 var(--mono, "IBM Plex Mono"), ui-monospace, monospace; letter-spacing: .04em; text-transform: uppercase; color: var(--muted); }
  .wr .label.th { font-family: var(--font); font-size: 12px; letter-spacing: 0; text-transform: none; }
  .wr .rule { height: 1px; background: var(--line); }
  .wr .row { display: flex; gap: 16px; align-items: stretch; }
  .wr .row > * { flex: 1 1 0; min-width: 0; }
  .wr .card { background: var(--card); border: 1px solid var(--line); border-radius: var(--radius); padding: 24px; display: flex; flex-direction: column; gap: 20px; }
  .wr .card.flush { padding: 0; gap: 0; overflow: hidden; }
  .wr .bh { display: flex; gap: 16px; align-items: flex-start; }
  .wr .bh .t { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .wr .bh h3 { margin: 0; font-size: 20px; font-weight: 600; line-height: 1.3; letter-spacing: -.01em; color: var(--title); }
  .wr .bh .cap { font-size: 13px; color: var(--muted); line-height: 1.5; }
  .wr .chip { display: inline-flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 999px; font-size: 12px; font-weight: 500; line-height: 1.4; white-space: nowrap; background: var(--cbg); color: var(--ctx); }
  .wr .chip::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--cdot); flex: none; }
  .wr .chip.sm { padding: 1px 7px; }
  /* header */
  .wr .hd { display: flex; flex-direction: column; gap: 16px; }
  .wr .hd .top { display: flex; justify-content: space-between; align-items: center; gap: 16px; }
  .wr .hd h1 { margin: 0; font-size: 48px; font-weight: 600; line-height: 1.15; letter-spacing: -.02em; color: var(--title); }
  .wr .hd .sub { font-size: 18px; line-height: 1.55; color: var(--muted); max-width: 880px; }
  .wr .hd .meta { display: flex; gap: 20px; flex-wrap: wrap; font: 11px/1.4 var(--mono), monospace; letter-spacing: .04em; }
  .wr .hd .meta span b { font-weight: 400; color: var(--faint); margin-right: 6px; }
  .wr .hd .meta span { color: var(--text); }
  /* stats */
  .wr .stats { display: flex; gap: 16px; }
  .wr .stats .card { flex: 1 1 0; gap: 10px; }
  .wr .stat-v { display: flex; align-items: baseline; gap: 6px; }
  .wr .stat-v b { font-size: 44px; font-weight: 600; line-height: 1.1; letter-spacing: -.02em; color: var(--vcol, var(--title)); }
  .wr .stat-v span { font-size: 18px; font-weight: 500; color: var(--muted); }
  .wr .stat-f { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font: 11px var(--mono), monospace; color: var(--muted); }
  .wr .stat-bar { height: 4px; border-radius: 2px; background: var(--soft); overflow: hidden; }
  .wr .stat-bar i { display: block; height: 100%; background: var(--vcol, var(--accent)); border-radius: 2px; }
  /* compare */
  .wr .cmp { display: flex; flex-direction: column; gap: 8px; }
  .wr .cmp-h { display: flex; justify-content: space-between; align-items: center; gap: 12px; font-size: 14px; font-weight: 500; color: var(--title); }
  .wr .bar { display: grid; grid-template-columns: 44px 1fr auto; gap: 12px; align-items: center; font: 11px var(--mono), monospace; color: var(--muted); }
  .wr .bar .track { height: 10px; border-radius: 999px; background: var(--soft); overflow: hidden; }
  .wr .bar .track i { display: block; height: 100%; border-radius: 999px; min-width: 6px; }
  .wr .bar .v { color: var(--title); font-weight: 500; min-width: 64px; text-align: right; }
  .wr .legend { display: flex; gap: 12px; font: 11px var(--mono), monospace; color: var(--text); }
  .wr .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .wr .legend i { width: 10px; height: 10px; border-radius: 3px; }
  /* chart */
  .wr svg.chart { width: 100%; height: auto; display: block; overflow: visible; }
  .wr .axis { display: flex; justify-content: space-between; font: 11px var(--mono), monospace; color: var(--muted); margin-top: -10px; }
  .wr .note { display: flex; gap: 8px; align-items: baseline; background: var(--page); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; font-size: 13px; }
  .wr .note .label { flex: none; }
  /* board */
  .wr .cols { display: flex; gap: 16px; align-items: stretch; }
  .wr .col { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 12px; padding: 14px; border-radius: 12px; background: var(--colbg, var(--page)); border: 1px solid var(--soft); }
  .wr .col-h { display: flex; align-items: center; gap: 8px; padding: 0 4px; font-size: 14px; font-weight: 600; color: var(--title); }
  .wr .col-h i { width: 8px; height: 8px; border-radius: 50%; background: var(--dot); }
  .wr .col-h small { font: 11px var(--mono), monospace; color: var(--faint); font-weight: 400; }
  .wr .bcard { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 16px; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .wr .bcard b { font-size: 14px; font-weight: 600; line-height: 1.45; color: var(--title); }
  .wr .bcard span { font-size: 13px; line-height: 1.5; color: var(--muted); }
  /* table */
  .wr .tbl { width: 100%; border-collapse: collapse; font-size: 13px; }
  .wr .tbl th { text-align: left; padding: 10px 16px; font: 500 11px var(--mono), monospace; letter-spacing: .04em; text-transform: uppercase; color: var(--muted); background: var(--page); white-space: nowrap; }
  .wr .tbl.ink th { background: var(--title); color: var(--card); }
  .wr .tbl.nohead thead { display: none; }
  .wr .tbl td { padding: 13px 16px; border-top: 1px solid var(--soft); color: var(--text); vertical-align: middle; line-height: 1.5; }
  .wr .tbl.compact td { padding: 8px 16px; }
  .wr .tbl.zebra tbody tr.r:nth-child(even) td { background: color-mix(in srgb, var(--page) 60%, var(--card)); }
  .wr .tbl td.first { color: var(--title); font-weight: 500; }
  .wr .tbl td.num, .wr .tbl th.num { text-align: right; font-family: var(--mono), monospace; font-size: 12.5px; }
  .wr .tbl td.link { font-family: var(--mono), monospace; font-size: 12.5px; color: var(--accent); }
  .wr .tbl tr.group td { background: var(--page); font-size: 12px; font-weight: 600; padding-top: 6px; padding-bottom: 6px; }
  .wr .tbl tr.total td { background: var(--page); font-weight: 600; color: var(--title); border-top: 1px solid var(--line); }
  .wr .tbl .pbar { display: inline-flex; align-items: center; gap: 8px; font: 12px var(--mono), monospace; color: var(--title); }
  .wr .tbl .pbar span { width: 90px; height: 6px; border-radius: 3px; background: var(--soft); overflow: hidden; display: inline-block; }
  .wr .tbl .pbar i { display: block; height: 100%; }
  .wr .card.flush .bh { padding: 24px 24px 16px; }
  /* timeline */
  .wr .tl { display: flex; flex-direction: column; }
  .wr .tl-i { display: grid; grid-template-columns: 48px 12px 1fr; gap: 12px; font-size: 13px; }
  .wr .tl-i .tm { font: 11px/1.9 var(--mono), monospace; color: var(--faint); }
  .wr .tl-i .rail { display: flex; flex-direction: column; align-items: center; padding-top: 6px; }
  .wr .tl-i .rail i { width: 9px; height: 9px; border-radius: 50%; flex: none; }
  .wr .tl-i .rail s { width: 1px; flex: 1; min-height: 14px; background: var(--line); margin: 4px 0; }
  .wr .tl-i p { padding-bottom: 14px; line-height: 1.6; }
  /* callout */
  .wr .co { border-radius: 12px; padding: 16px; display: flex; flex-direction: column; gap: 4px; background: var(--cbg); border: 1px solid var(--cbd); color: var(--ctx); font-size: 13px; line-height: 1.55; }
  .wr .co b { font-size: 14px; font-weight: 600; }
  /* checklist */
  .wr .ck { display: flex; flex-direction: column; gap: 10px; }
  .wr .ck-i { display: flex; align-items: center; gap: 10px; font-size: 14px; }
  .wr .ck-i .box { width: 18px; height: 18px; border-radius: 5px; flex: none; display: grid; place-items: center; border: 1.5px solid var(--line); background: var(--card); }
  .wr .ck-i.on .box { background: var(--onc, var(--ok)); border-color: var(--onc, var(--ok)); }
  .wr .ck-i span { flex: 1; color: var(--title); }
  .wr .ck-i small { font: 11px var(--mono), monospace; color: var(--faint); }
  /* flow */
  .wr .fl { display: flex; align-items: center; }
  .wr .fl-s { flex: 1 1 0; min-width: 0; background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 12px 14px; display: flex; flex-direction: column; gap: 4px; }
  .wr .fl-s b { font-size: 14px; font-weight: 500; color: var(--title); }
  .wr .fl-a { flex: none; width: 34px; display: grid; place-items: center; color: var(--faint); }
  /* text */
  .wr .md { font-size: 15px; line-height: 1.7; color: var(--text); display: flex; flex-direction: column; gap: 10px; }
  .wr .md ul { margin: 0; padding-left: 20px; }
  .wr .md code { font: 13px var(--mono), monospace; background: var(--page); padding: 1px 5px; border-radius: 4px; }
  .wr .md a { color: var(--accent); }
  .wr .img img { width: 100%; border-radius: 10px; display: block; border: 1px solid var(--line); }
  .wr .img figcaption { font: 12px var(--font); color: var(--muted); margin-top: 8px; }
  .wr .ft { display: flex; justify-content: space-between; font: 11px var(--mono), monospace; letter-spacing: .04em; color: var(--faint); text-transform: uppercase; }
  `;

  const THAI = /[฀-๿]/;
  const lab = (t) => `<p class="label${THAI.test(t) ? " th" : ""}">◇ ${esc(t)}</p>`;

  function make(theme0) {
    // a value that may name a token ("accent") → its color
    const T = (k, style) => {
      let v = style?.[k] ?? theme0[k];
      for (let i = 0; i < 4 && typeof v === "string" && theme0[v] !== undefined && !v.startsWith("#"); i++) v = theme0[v];
      return v;
    };
    const col = (v, style) => (v == null ? null : typeof v === "string" && !v.startsWith("#") && theme0[v] !== undefined ? T(v, style) : v);
    const KIND = (kind, style) => {
      const k = ["ok", "warn", "bad", "info", "neutral"].includes(kind) ? kind : "neutral";
      const base = col(style?.chip?.[k]?.dot ?? (k === "neutral" ? "neutral" : k), style);
      return {
        dot: base,
        bg: style?.chip?.[k]?.bg ?? `color-mix(in srgb, ${base} 13%, ${T("card")})`,
        text: style?.chip?.[k]?.text ?? (k === "neutral" ? T("text") : `color-mix(in srgb, ${base} 72%, #000)`),
        line: `color-mix(in srgb, ${base} 30%, ${T("card")})`,
      };
    };
    const chip = (text, kind = "neutral", style, sm) => {
      const c = KIND(kind, style);
      return `<span class="chip${sm ? " sm" : ""}" style="--cbg:${c.bg};--cdot:${c.dot};--ctx:${c.text}">${esc(text)}</span>`;
    };
    const chipOf = (v, style) => {
      if (v == null || v === "") return "";
      if (Array.isArray(v)) return chip(v[0], v[1], style);
      if (typeof v === "object") return chip(v.text ?? v.chip ?? "", v.kind, style);
      return chip(v, "neutral", style);
    };
    const head = (b, right = "") => (b.title || b.eyebrow || b.caption || right
      ? `<div class="bh"><div class="t">${b.eyebrow ? lab(b.eyebrow) : ""}${b.title ? `<h3>${esc(b.title)}</h3>` : ""}${b.caption ? `<p class="cap">${esc(b.caption)}</p>` : ""}</div>${right}</div>` : "");
    const vars = (style) => {
      if (!style) return "";
      const out = [];
      for (const k of ["page", "card", "line", "soft", "title", "text", "muted", "faint", "accent", "ok", "warn", "bad", "radius"]) if (style[k] != null) out.push(`--${k}:${k === "radius" ? `${style[k]}px` : col(style[k], style)}`);
      return out.length ? ` style="${out.join(";")}"` : "";
    };
    const series = (style) => (style?.series ?? theme0.series ?? []).map((c) => col(c, style));

    const B = {
      header(b) {
        const meta = Array.isArray(b.meta) ? b.meta : Object.entries(b.meta ?? {});
        return `<div class="hd"${vars(b.style)}>
          ${b.eyebrow || b.status ? `<div class="top">${b.eyebrow ? lab(b.eyebrow) : "<span></span>"}${b.status ? chipOf(b.status, b.style) : ""}</div>` : ""}
          <h1>${esc(b.title)}</h1>${b.subtitle ? `<p class="sub">${esc(b.subtitle)}</p>` : ""}
          ${meta.length ? `<div class="meta">${meta.map(([k, v]) => `<span><b>${esc(k)}</b>${esc(v)}</span>`).join("")}</div>` : ""}
        </div>`;
      },
      stats(b) {
        return `<div class="stats"${vars(b.style)}>${(b.items ?? []).map((it) => {
          const c = col(it.color, b.style);
          const d = it.delta ? chip(it.delta, it.kind ?? (/^[↓-]/.test(it.delta) ? "ok" : /^[↑+]/.test(it.delta) ? "ok" : "neutral"), b.style) : "";
          const bar = typeof it.bar === "number" ? `<div class="stat-bar"><i style="width:${Math.max(2, Math.min(100, it.bar * 100))}%"></i></div>` : "";
          return `<div class="card"${c ? ` style="--vcol:${c}"` : ""}>${lab(it.label ?? "")}<div class="stat-v"><b>${esc(it.value)}</b>${it.unit ? `<span>${esc(it.unit)}</span>` : ""}</div>${bar}${d || it.note ? `<div class="stat-f">${d}${it.note ? `<span>${esc(it.note)}</span>` : ""}</div>` : ""}</div>`;
        }).join("")}</div>`;
      },
      compare(b) {
        const before = col(b.style?.before ?? "before", b.style), after = col(b.style?.after ?? "after", b.style);
        const rows = b.rows ?? [];
        const max = Math.max(1e-9, ...rows.flatMap((r) => [Math.abs(+r.before || 0), Math.abs(+r.after || 0)]));
        const fmt = (v, t) => t ?? (v == null ? "" : `${v}${b.unit ? ` ${b.unit}` : ""}`);
        const legend = `<div class="legend"><span><i style="background:${before}"></i>${esc(b.labels?.[0] ?? "before")}</span><span><i style="background:${after}"></i>${esc(b.labels?.[1] ?? "after")}</span></div>`;
        return `<div class="card"${vars(b.style)}>${head(b, legend)}${rows.map((r) => {
          const bv = +r.before || 0, av = +r.after || 0;
          const better = r.better ?? b.better ?? "lower";
          let change = r.change;
          if (change == null && bv) { const p = Math.round(((av - bv) / Math.abs(bv)) * 100); change = `${p > 0 ? "↑" : p < 0 ? "↓" : ""} ${Math.abs(p)}%`; }
          const good = r.kind ?? (av === bv ? "neutral" : (av < bv) === (better === "lower") ? "ok" : "bad");
          const w = (v) => `${Math.max(1, (Math.abs(v) / max) * 100)}%`;
          return `<div class="cmp"><div class="cmp-h"><span>${esc(r.label)}</span>${change ? chip(change, good, b.style) : ""}</div>
            <div class="bar"><span>${esc(b.labels?.[0] ?? "before")}</span><div class="track"><i style="width:${w(bv)};background:${col(r.colors?.[0], b.style) ?? before}"></i></div><span class="v">${esc(fmt(r.before, r.beforeText))}</span></div>
            <div class="bar"><span>${esc(b.labels?.[1] ?? "after")}</span><div class="track"><i style="width:${w(av)};background:${col(r.color ?? r.colors?.[1], b.style) ?? after}"></i></div><span class="v">${esc(fmt(r.after, r.afterText))}</span></div></div>`;
        }).join("")}</div>`;
      },
      chart(b) {
        const W = 600, H = b.height ?? 220, pad = 12;
        const ss = (b.series ?? []).map((s, i) => ({ ...s, color: col(s.color, b.style) ?? series(b.style)[i % Math.max(1, series(b.style).length)] ?? T("accent") }));
        const all = ss.flatMap((s) => s.values ?? []).map(Number).filter((v) => !Number.isNaN(v));
        const lo = b.min ?? Math.min(0, ...all), hi = b.max ?? Math.max(...all, 1);
        const n = Math.max(...ss.map((s) => (s.values ?? []).length), (b.x ?? []).length, 1);
        const y = (v) => pad + (1 - (v - lo) / (hi - lo || 1)) * (H - pad * 2);
        const grid = col(b.style?.grid ?? "grid", b.style);
        let g = [0, 1, 2, 3].map((i) => `<line x1="0" x2="${W}" y1="${pad + (i * (H - pad * 2)) / 3}" y2="${pad + (i * (H - pad * 2)) / 3}" stroke="${grid}"/>`).join("");
        if ((b.kind ?? "line") === "bars") {
          const slot = W / n, bw = Math.min(48, (slot * 0.7) / Math.max(1, ss.length));
          ss.forEach((s, si) => (s.values ?? []).forEach((v, i) => {
            const x = i * slot + (slot - bw * ss.length) / 2 + si * bw, top = y(+v), base = y(Math.max(lo, 0));
            g += `<rect x="${x.toFixed(1)}" y="${Math.min(top, base).toFixed(1)}" width="${(bw - 3).toFixed(1)}" height="${Math.max(1, Math.abs(base - top)).toFixed(1)}" rx="5" fill="${s.color}"${b.highlight != null && i !== b.highlight ? ' fill-opacity=".35"' : ""}/>`;
          }));
        } else {
          const x = (i) => pad + (i * (W - pad * 2)) / Math.max(1, n - 1);
          ss.forEach((s, si) => {
            const vs = (s.values ?? []).map(Number);
            const d = vs.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
            if (si === 0 && (b.style?.area ?? "on") !== "none" && vs.length) g += `<path d="${d} L${x(vs.length - 1)} ${H - pad} L${x(0)} ${H - pad} Z" fill="${s.color}" fill-opacity=".08"/>`;
            g += `<path d="${d}" fill="none" stroke="${s.color}" stroke-width="2.5" stroke-linejoin="round" stroke-linecap="round"/>`;
            g += vs.map((v, i) => `<circle cx="${x(i)}" cy="${y(v)}" r="${i === vs.length - 1 ? 5 : 3.5}" fill="${i === vs.length - 1 ? s.color : T("card")}" stroke="${s.color}" stroke-width="2"/>`).join("");
          });
        }
        const legend = ss.length > 1 ? `<div class="legend">${ss.map((s) => `<span><i style="background:${s.color}"></i>${esc(s.name ?? "")}</span>`).join("")}</div>` : b.badge ? chipOf(b.badge, b.style) : "";
        return `<div class="card"${vars(b.style)}>${head(b, legend)}<svg class="chart" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" style="aspect-ratio:${W}/${H}">${g}</svg>
          ${b.x?.length ? `<div class="axis">${b.x.map((v) => `<span>${esc(v)}</span>`).join("")}</div>` : ""}
          ${b.note ? `<div class="note"><span class="label">${esc(b.noteLabel ?? "note")}</span><span>${esc(b.note)}</span></div>` : ""}</div>`;
      },
      board(b) {
        const cols = b.columns ?? [];
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="cols">${cols.map((c, i) => {
          const st = b.style?.columns?.[i] ?? {};
          const dot = col(st.dot ?? c.color, b.style) ?? KIND(c.kind ?? "neutral", b.style).dot;
          return `<div class="col" style="--dot:${dot}${st.bg ? `;--colbg:${col(st.bg, b.style)}` : ""}"><div class="col-h"><i></i>${esc(c.title)}<small>${(c.cards ?? []).length}</small></div>${(c.cards ?? []).map((k) => `<div class="bcard">${k.status ? chipOf(k.status, b.style) : ""}<b>${esc(k.title)}</b>${k.body ? `<span>${esc(k.body)}</span>` : ""}</div>`).join("")}</div>`;
        }).join("")}</div></div>`;
      },
      table(b) {
        const preset = b.preset ?? "basic";
        const rows = b.rows ?? [];
        const cols = (b.columns ?? []).map((c, i) => {
          const o = typeof c === "string" ? { name: c } : { ...c };
          if (!o.kind) {
            const sample = rows.find((r) => Array.isArray(r) && r[i] != null && r[i] !== "")?.[i];
            o.kind = typeof sample === "boolean" ? "check" : sample && typeof sample === "object" ? (preset === "compare" && i === (b.columns.length - 1) ? "delta" : "chip")
              : preset === "progress" && typeof sample === "number" && sample <= 1 ? "bar"
              : (preset === "numbers" || preset === "compare") && i > 0 ? "num" : typeof sample === "number" ? "num" : "text";
          }
          return o;
        });
        const cell = (c, v, i) => {
          const cls = [i === 0 ? "first" : "", c.kind === "num" ? "num" : "", c.kind === "link" ? "link" : ""].filter(Boolean).join(" ");
          const w = c.width ? ` style="width:${c.width}px"` : "";
          let inner;
          if (v == null || v === "") inner = "";
          else if (c.kind === "chip" || c.kind === "delta") inner = chipOf(v, b.style);
          else if (c.kind === "check") inner = v ? `<svg width="16" height="16" viewBox="0 0 18 18"><rect width="18" height="18" rx="5" fill="${T("ok")}"/><path d="m5 9.2 2.6 2.6L13 6.4" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>` : `<svg width="16" height="16" viewBox="0 0 18 18"><rect x=".75" y=".75" width="16.5" height="16.5" rx="4.5" fill="none" stroke="${T("line")}" stroke-width="1.5"/></svg>`;
          else if (c.kind === "bar") { const p = typeof v === "number" ? v : parseFloat(v) / 100; inner = `<span class="pbar"><span><i style="width:${Math.max(3, Math.min(100, p * 100))}%;background:${p >= 1 ? T("ok") : T("accent")}"></i></span>${Math.round(p * 100)}%</span>`; }
          else if (v && typeof v === "object") inner = `<span${v.color ? ` style="color:${col(v.color, b.style)}"` : ""}>${esc(v.text ?? "")}</span>`;
          else inner = esc(v);
          return `<td class="${cls}"${w}>${inner}</td>`;
        };
        const header = b.header ?? "soft";
        const cls = ["tbl", b.compact ? "compact" : "", b.zebra ? "zebra" : "", header === "ink" ? "ink" : "", header === "none" ? "nohead" : ""].filter(Boolean).join(" ");
        const body = rows.map((r) => (typeof r === "string" ? `<tr class="group"><td colspan="${cols.length}">${esc(r)}</td></tr>` : `<tr class="r">${cols.map((c, i) => cell(c, r[i], i)).join("")}</tr>`)).join("");
        const total = b.total ? `<tr class="total">${cols.map((c, i) => `<td class="${c.kind === "num" ? "num" : ""}">${esc(b.total[i] ?? "")}</td>`).join("")}</tr>` : "";
        return `<div class="card flush"${vars(b.style)}>${head(b)}<table class="${cls}"><thead><tr>${cols.map((c) => `<th class="${c.kind === "num" ? "num" : ""}">${esc(c.name)}</th>`).join("")}</tr></thead><tbody>${body}${total}</tbody></table></div>`;
      },
      timeline(b) {
        const it = b.items ?? [];
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="tl">${it.map((x, i) => `<div class="tl-i"><span class="tm">${esc(x.time ?? "")}</span><span class="rail"><i style="background:${col(x.color, b.style) ?? KIND(x.kind ?? "info", b.style).dot}"></i>${i < it.length - 1 ? "<s></s>" : ""}</span><p>${esc(x.text)}</p></div>`).join("")}</div></div>`;
      },
      callout(b) {
        const c = KIND(b.kind ?? "info", b.style);
        const st = b.style ?? {};
        return `<div class="co" style="--cbg:${col(st.bg, st) ?? c.bg};--cbd:${col(st.border, st) ?? c.line};--ctx:${col(st.text, st) ?? c.text}">${b.title ? `<b>${esc(b.title)}</b>` : ""}${b.text ? `<span>${esc(b.text)}</span>` : ""}</div>`;
      },
      checklist(b) {
        const on = col(b.style?.on, b.style);
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="ck"${on ? ` style="--onc:${on}"` : ""}>${(b.items ?? []).map((x) => `<div class="ck-i${x.done ? " on" : ""}"><span class="box">${x.done ? '<svg width="12" height="12" viewBox="0 0 12 12"><path d="m2.5 6.2 2.3 2.3 4.7-4.9" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>' : ""}</span><span>${esc(x.text)}</span>${x.note ? `<small>${esc(x.note)}</small>` : ""}</div>`).join("")}</div></div>`;
      },
      flow(b) {
        const s = b.steps ?? [];
        const arrow = `<span class="fl-a"><svg width="28" height="12" viewBox="0 0 28 12"><path d="M2 6h22m-4-4 4 4-4 4" fill="none" stroke="${col(b.style?.arrow, b.style) ?? T("faint")}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg></span>`;
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="fl">${s.map((x, i) => `${i ? arrow : ""}<div class="fl-s"${x.kind ? ` style="border-color:${KIND(x.kind, b.style).line}"` : ""}>${x.actor ? lab(x.actor) : ""}<b>${esc(x.text)}</b></div>`).join("")}</div></div>`;
      },
      text(b) {
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="md">${md(b.text)}</div></div>`;
      },
      image(b, ctx) {
        const src = /^(https?:|data:)/.test(b.src ?? "") ? b.src : ctx.fileUrl ? ctx.fileUrl(b.src) : b.src;
        return `<figure class="card img" style="margin:0"${vars(b.style)}>${head(b)}<img src="${esc(src)}" alt="${esc(b.alt ?? b.caption ?? "")}">${b.caption ? `<figcaption>${esc(b.caption)}</figcaption>` : ""}</figure>`;
      },
      tasks(b) {
        const items = b.items ?? [];
        const K = { todo: ["To do", "neutral"], doing: ["Doing", "info"], review: ["Review", "warn"], done: ["Done", "ok"], skipped: ["Skipped", "neutral"] };
        const count = (s) => items.filter((t) => t.status === s).length;
        const f = b.filter ?? {};
        const title = b.title ?? (typeof f === "string" ? f : f.from ?? f.label ?? "Tasks");
        const badges = `<div style="display:flex;gap:8px">${chip(`${items.length} task${items.length === 1 ? "" : "s"}`, "neutral", b.style)}${count("done") ? chip(`${count("done")} done`, "ok", b.style) : ""}${count("review") ? chip(`${count("review")} to review`, "warn", b.style) : ""}</div>`;
        const where = (t) => t.change?.where || (t.links ?? [])[0] || "";
        return `<div class="card flush"${vars(b.style)}>${head({ ...b, title }, badges)}<table class="tbl compact"><thead><tr><th>Task</th><th>Status</th><th>Where</th></tr></thead><tbody>${items.map((t) => `<tr class="r"><td class="first">${esc(t.title)}</td><td style="width:130px">${chip(...(K[t.status] ?? [t.status, "neutral"]), b.style)}</td><td class="link">${esc(where(t))}</td></tr>`).join("") || `<tr><td colspan="3" style="color:var(--muted)">No tasks match.</td></tr>`}</tbody></table></div>`;
      },
      divider() { return `<div class="rule"></div>`; },
    };
    return { B, T };
  }

  // Draws `data` ({report, theme}) into `host`. ctx: { fileUrl(path) } for image paths in the project.
  function render(host, data, ctx = {}) {
    const theme = data.theme ?? {};
    const { B, T } = make(theme);
    if (!document.getElementById("wr-css")) { const s = document.createElement("style"); s.id = "wr-css"; s.textContent = CSS; document.head.append(s); }
    const r = data.report ?? {};
    const root = document.createElement("div");
    root.className = "wr";
    root.style.cssText = ["page", "card", "line", "soft", "title", "text", "muted", "faint", "accent", "ok", "warn", "bad"].map((k) => `--${k}:${T(k)}`).join(";")
      + `;--radius:${theme.radius ?? 14}px;--font:"${theme.font ?? "IBM Plex Sans Thai"}";--mono:"${theme.mono ?? "IBM Plex Mono"}";width:${theme.width ?? 1440}px`;
    const one = (b) => { try { return (B[b?.type] ?? (() => `<div class="co" style="--cbg:#FBE7E4;--cbd:#F0C8C2;--ctx:#A42C22">Unknown block “${esc(b?.type)}”</div>`))(b ?? {}, ctx); } catch (e) { return `<div class="co" style="--cbg:#FBE7E4;--cbd:#F0C8C2;--ctx:#A42C22">Couldn't draw a ${esc(b?.type)} block: ${esc(e.message)}</div>`; } };
    const rows = r.rows ?? [];
    // a report without a header block gets one from its title
    const hasHeader = rows.some((x) => (Array.isArray(x) ? x : [x]).some((b) => b?.type === "header"));
    let html = hasHeader ? "" : B.header({ title: r.title, subtitle: r.subtitle, eyebrow: r.eyebrow, status: r.status, meta: r.meta }) + `<div class="rule"></div>`;
    for (const row of rows) {
      if (Array.isArray(row)) html += `<div class="row">${row.map((b) => `<div style="flex:${b?.span ?? 1} 1 0;min-width:0;display:flex;flex-direction:column">${one(b).replace(/^<div class="card/, '<div style="flex:1" class="card')}</div>`).join("")}</div>`;
      else html += one(row);
    }
    if (r.footer !== false) html += `<div class="rule"></div><div class="ft"><span>${esc(r.footer ?? `Made with wridge${data.name ? ` · docs/reports/${data.name}.json` : ""}`)}</span><span>${esc(r.updated ?? "")}</span></div>`;
    root.innerHTML = html;
    host.replaceChildren(root);
    return root;
  }

  window.wrReport = { render, md, esc };
})();
