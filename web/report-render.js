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
  .wr .label { margin: 0; font: var(--fw-label, 500) var(--fs-label, calc(13px * var(--s, 1)))/1.4 var(--mono, "IBM Plex Mono"), ui-monospace, monospace; letter-spacing: .04em; text-transform: uppercase; color: var(--muted); }
  .wr .label.th { font-family: var(--font); font-size: calc(14px * var(--s, 1)); letter-spacing: 0; text-transform: none; }
  .wr .rule { height: 1px; background: var(--line); }
  .wr .row { display: flex; gap: 16px; align-items: stretch; }
  .wr .row > * { flex: 1 1 0; min-width: 0; }
  .wr .card { background: var(--card); border: 1px solid var(--line); border-radius: var(--radius); padding: 24px; display: flex; flex-direction: column; gap: 20px; }
  .wr .card.flush { padding: 0; gap: 0; overflow: hidden; }
  .wr .bh { display: flex; gap: 16px; align-items: flex-start; }
  .wr .bh .t { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .wr .bh h3 { margin: 0; font-size: var(--fs-heading, calc(24px * var(--s, 1))); font-weight: var(--fw-heading, 600); line-height: 1.3; letter-spacing: -.01em; color: var(--title); }
  .wr .bh .cap { font-size: calc(16px * var(--s, 1)); color: var(--muted); line-height: 1.5; }
  .wr .chip { display: inline-flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 999px; font-size: calc(14px * var(--s, 1)); font-weight: 500; line-height: 1.4; white-space: nowrap; background: var(--cbg); color: var(--ctx); }
  .wr .chip::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--cdot); flex: none; }
  .wr .chip.sm { padding: 1px 7px; }
  /* header */
  .wr .hd { display: flex; flex-direction: column; gap: 16px; }
  .wr .hd .top { display: flex; justify-content: space-between; align-items: center; gap: 16px; }
  .wr .hd h1 { margin: 0; font-size: var(--fs-title, calc(48px * var(--s, 1))); font-weight: var(--fw-title, 600); line-height: 1.15; letter-spacing: -.02em; color: var(--title); }
  .wr .hd .sub { font-size: var(--fs-subtitle, calc(20px * var(--s, 1))); line-height: 1.55; color: var(--muted); max-width: 880px; }
  .wr .hd .meta { display: flex; gap: 20px; flex-wrap: wrap; font: calc(13px * var(--s, 1))/1.4 var(--mono), monospace; letter-spacing: .04em; }
  .wr .hd .meta span b { font-weight: 400; color: var(--faint); margin-right: 6px; }
  .wr .hd .meta span { color: var(--text); }
  /* stats */
  .wr .stats { display: flex; gap: 16px; }
  .wr .stats .card { flex: 1 1 0; gap: 10px; }
  .wr .stat-v { display: flex; align-items: baseline; gap: 6px; }
  .wr .stat-v b { font-size: var(--fs-number, calc(44px * var(--s, 1))); font-weight: var(--fw-number, 600); line-height: 1.1; letter-spacing: -.02em; color: var(--vcol, var(--sval, var(--title))); }
  .wr .stat-v span { font-size: calc(20px * var(--s, 1)); font-weight: 500; color: var(--muted); }
  .wr .stat-f { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); }
  .wr .stat-bar { height: 4px; border-radius: 2px; background: var(--soft); overflow: hidden; }
  .wr .stat-bar i { display: block; height: 100%; background: var(--vcol, var(--sbar, var(--accent))); border-radius: 2px; }
  /* compare */
  .wr .cmp { display: flex; flex-direction: column; gap: 8px; }
  .wr .cmp-h { display: flex; justify-content: space-between; align-items: center; gap: 12px; font-size: calc(17px * var(--s, 1)); font-weight: 500; color: var(--title); }
  .wr .bar { display: grid; grid-template-columns: 44px 1fr auto; gap: 12px; align-items: center; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); }
  .wr .bar .track { height: 10px; border-radius: 999px; background: var(--soft); overflow: hidden; }
  .wr .bar .track i { display: block; height: 100%; border-radius: 999px; min-width: 6px; }
  .wr .bar .v { color: var(--title); font-weight: 500; min-width: 64px; text-align: right; }
  .wr .legend { display: flex; gap: 12px; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--text); }
  .wr .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .wr .legend i { width: 10px; height: 10px; border-radius: 3px; }
  /* chart */
  .wr svg.chart { width: 100%; height: auto; display: block; overflow: visible; }
  .wr .axis { display: flex; justify-content: space-between; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); margin-top: -10px; }
  .wr .note { display: flex; gap: 8px; align-items: baseline; background: var(--page); border: 1px solid var(--line); border-radius: 10px; padding: 10px 14px; font-size: calc(16px * var(--s, 1)); }
  .wr .note .label { flex: none; }
  /* board */
  .wr .cols { display: flex; gap: 16px; align-items: stretch; }
  .wr .col { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 12px; padding: 14px; border-radius: 12px; background: var(--colbg, var(--page)); border: 1px solid var(--soft); }
  .wr .col-h { display: flex; align-items: center; gap: 8px; padding: 0 4px; font-size: calc(17px * var(--s, 1)); font-weight: 600; color: var(--title); }
  .wr .col-h i { width: 8px; height: 8px; border-radius: 50%; background: var(--dot); }
  .wr .col-h small { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--faint); font-weight: 400; }
  .wr .bcard { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 16px; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .wr .bcard b { font-size: calc(17px * var(--s, 1)); font-weight: 600; line-height: 1.45; color: var(--title); }
  .wr .bcard span { font-size: calc(16px * var(--s, 1)); line-height: 1.5; color: var(--muted); }
  /* table */
  .wr .tbl { width: 100%; border-collapse: collapse; font-size: var(--fs-body, calc(16px * var(--s, 1))); }
  .wr .tbl th { text-align: left; padding: 10px 16px; font: 500 calc(13px * var(--s, 1)) var(--mono), monospace; letter-spacing: .04em; text-transform: uppercase; color: var(--th-tx, var(--muted)); background: var(--th-bg, var(--page)); white-space: nowrap; }
  .wr .tbl.ink th { background: var(--th-bg, var(--title)); color: var(--th-tx, var(--card)); }
  .wr .tbl.nohead thead { display: none; }
  .wr .tbl td { padding: 13px 16px; border-top: 1px solid var(--rule, var(--soft)); color: var(--text); vertical-align: middle; line-height: 1.5; background: var(--row, transparent); }
  .wr .tbl tr.r[style*="--row"] td { background: var(--row) !important; }
  .wr .tbl.compact td { padding: 8px 16px; }
  .wr .tbl.zebra tbody tr.r:nth-child(even) td { background: var(--zebra, color-mix(in srgb, var(--page) 60%, var(--card))); }
  .wr .tbl td.first { color: var(--title); font-weight: 500; }
  .wr .tbl td.num, .wr .tbl th.num { text-align: right; font-family: var(--mono), monospace; font-size: calc(15px * var(--s, 1)); }
  .wr .tbl td.link { font-family: var(--mono), monospace; font-size: calc(15px * var(--s, 1)); color: var(--accent); }
  .wr .tbl tr.group td { background: var(--page); font-size: calc(14px * var(--s, 1)); font-weight: 600; padding-top: 6px; padding-bottom: 6px; }
  .wr .tbl tr.total td { background: var(--total, var(--page)); font-weight: 600; color: var(--title); border-top: 1px solid var(--line); }
  .wr .tbl .pbar { display: inline-flex; align-items: center; gap: 8px; font: calc(14px * var(--s, 1)) var(--mono), monospace; color: var(--title); }
  .wr .tbl .pbar span { width: 90px; height: 6px; border-radius: 3px; background: var(--soft); overflow: hidden; display: inline-block; }
  .wr .tbl .pbar i { display: block; height: 100%; }
  .wr .card.flush .bh { padding: 24px 24px 16px; }
  /* timeline */
  .wr .tl { display: flex; flex-direction: column; }
  .wr .tl-i { display: grid; grid-template-columns: 56px 12px 1fr; gap: 12px; font-size: calc(16px * var(--s, 1)); }
  .wr .tl-i .tm { font: calc(13px * var(--s, 1))/1.9 var(--mono), monospace; color: var(--tl-time, var(--faint)); }
  .wr .tl-i .rail { display: flex; flex-direction: column; align-items: center; padding-top: 6px; }
  .wr .tl-i .rail i { width: 9px; height: 9px; border-radius: 50%; flex: none; }
  .wr .tl-i .rail s { width: 1px; flex: 1; min-height: 14px; background: var(--tl-line, var(--line)); margin: 4px 0; }
  .wr .tl-i p { padding-bottom: 14px; line-height: 1.6; }
  /* callout */
  .wr .co { border-radius: 12px; padding: 16px; display: flex; flex-direction: column; gap: 4px; background: var(--cbg); border: 1px solid var(--cbd); color: var(--ctx); font-size: calc(16px * var(--s, 1)); line-height: 1.55; }
  .wr .co b { font-size: calc(17px * var(--s, 1)); font-weight: 600; }
  /* checklist */
  .wr .ck { display: flex; flex-direction: column; gap: 10px; }
  .wr .ck-i { display: flex; align-items: center; gap: 10px; font-size: calc(16px * var(--s, 1)); }
  .wr .ck-i .box { width: 18px; height: 18px; border-radius: 5px; flex: none; display: grid; place-items: center; border: 1.5px solid var(--line); background: var(--card); }
  .wr .ck-i.on .box { background: var(--onc, var(--ok)); border-color: var(--onc, var(--ok)); }
  .wr .ck-i span { flex: 1; color: var(--title); }
  .wr .ck-i small { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--faint); }
  /* flow */
  .wr .fl { display: flex; align-items: center; }
  .wr .fl-s { flex: 1 1 0; min-width: 0; background: var(--fl-bg, var(--card)); border: 1px solid var(--fl-bd, var(--line)); border-radius: 10px; padding: 12px 14px; display: flex; flex-direction: column; gap: 4px; }
  .wr .fl-s b { font-size: calc(17px * var(--s, 1)); font-weight: 500; color: var(--title); }
  .wr .fl-a { flex: none; width: 34px; display: grid; place-items: center; color: var(--faint); }
  /* text */
  .wr .md { font-size: var(--fs-body, calc(18px * var(--s, 1))); font-weight: var(--fw-body, 400); line-height: 1.7; color: var(--text); display: flex; flex-direction: column; gap: 10px; }
  .wr .md ul { margin: 0; padding-left: 20px; }
  .wr .md code { font: calc(16px * var(--s, 1)) var(--mono), monospace; background: var(--page); padding: 1px 5px; border-radius: 4px; }
  .wr .md a { color: var(--accent); }
  .wr .img img { width: 100%; border-radius: 10px; display: block; border: 1px solid var(--line); }
  .wr .img figcaption { font: calc(14px * var(--s, 1)) var(--font); color: var(--muted); margin-top: 8px; }
  .wr .fl-s .label { color: var(--fl-actor, var(--muted)); }
  /* tasks block: board and list views */
  .wr .tb-cols { display: flex; gap: 12px; padding: 0 24px 24px; }
  .wr .tb-col { flex: 1 1 0; min-width: 0; display: flex; flex-direction: column; gap: 8px; padding: 10px; border-radius: 10px; background: var(--page); border: 1px solid var(--soft); }
  .wr .tb-col h4 { margin: 0; display: flex; align-items: center; gap: 7px; font-size: calc(16px * var(--s, 1)); font-weight: 600; color: var(--title); }
  .wr .tb-col h4 i { width: 7px; height: 7px; border-radius: 50%; } .wr .tb-col h4 small { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--faint); font-weight: 400; }
  .wr .tb-card { background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 10px; font-size: calc(15px * var(--s, 1)); font-weight: 500; color: var(--title); line-height: 1.45; }
  .wr .tb-card small { display: block; margin-top: 4px; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); font-weight: 400; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wr .tb-list { display: flex; flex-direction: column; padding: 0 24px 16px; }
  .wr .tb-li { display: flex; align-items: center; gap: 10px; padding: 9px 0; border-top: 1px solid var(--soft); font-size: calc(16px * var(--s, 1)); color: var(--title); }
  .wr .tb-li i { width: 8px; height: 8px; border-radius: 50%; flex: none; } .wr .tb-li span { flex: 1; min-width: 0; } .wr .tb-li small { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); }
  .wr .ft { display: flex; justify-content: space-between; font: calc(13px * var(--s, 1)) var(--mono), monospace; letter-spacing: .04em; color: var(--faint); text-transform: uppercase; }
  
  .wr [style*="--font:"] { font-family: var(--font), "Segoe UI", system-ui, sans-serif; }
  .wr .card.grow { flex: 1; }
  /* density: compact */
  .wr.compact { padding: 40px; gap: 24px; }
  .wr.compact .row, .wr.compact .stats, .wr.compact .cols { gap: 12px; }
  .wr.compact .card, .wr .card.compact { padding: 16px; gap: 14px; }
  .wr.compact .card.flush, .wr .card.flush.compact { padding: 0; }
  .wr.compact .card.flush .bh, .wr .card.flush.compact .bh { padding: 16px 16px 10px; }
  .wr.compact .tbl td, .wr .compact .tbl td { padding: 8px 14px; }
  .wr.compact .tbl th, .wr .compact .tbl th { padding: 8px 14px; }
  .wr.compact .hd { gap: 10px; }
  .wr.compact .col, .wr .compact .col { padding: 10px; gap: 8px; }
  .wr.compact .bcard, .wr .compact .bcard { padding: 12px; }
  /* charts */
  .wr .chart-host { width: 100%; min-height: 40px; }
  .wr svg.chart { display: block; overflow: visible; }
  .wr svg text { font: calc(13px * var(--s, 1)) var(--mono), monospace; fill: var(--muted); }
  .wr svg text.v { fill: var(--title); font-weight: 500; }
  .wr svg text.m { font-weight: 500; }
  .wr .hb { display: flex; flex-direction: column; gap: 10px; }
  .wr .hb-r { display: grid; grid-template-columns: minmax(120px, 34%) 1fr auto; gap: 12px; align-items: center; font-size: calc(16px * var(--s, 1)); }
  .wr .hb-l { color: var(--title); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wr .hb-l em { font: normal calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--faint); margin-right: 8px; }
  .wr .hb-t { height: calc(14px * var(--s, 1)); border-radius: 999px; background: var(--soft); overflow: hidden; }
  .wr .hb-t i { display: block; height: 100%; border-radius: 999px; }
  .wr .hb-v { font: 500 calc(14px * var(--s, 1)) var(--mono), monospace; color: var(--title); min-width: 56px; text-align: right; }
  .wr .dn { display: flex; align-items: center; gap: 28px; }
  .wr .dn-g { position: relative; flex: none; }
  .wr .dn-g svg { display: block; }
  .wr .dn-c { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; }
  .wr .dn-c b { font-size: calc(26px * var(--s, 1)); font-weight: 600; color: var(--title); letter-spacing: -.02em; line-height: 1.1; }
  .wr .dn-c span { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); text-transform: uppercase; letter-spacing: .04em; }
  .wr .dn-l { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 10px; }
  .wr .dn-i { display: grid; grid-template-columns: 10px 1fr auto 44px; gap: 10px; align-items: center; font-size: calc(16px * var(--s, 1)); }
  .wr .dn-i i { width: 10px; height: 10px; border-radius: 3px; }
  .wr .dn-i span { color: var(--title); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .wr .dn-i b { font: 500 calc(14px * var(--s, 1)) var(--mono), monospace; color: var(--title); }
  .wr .dn-i small { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); text-align: right; }
  .wr .gg { display: flex; gap: 16px; justify-content: space-around; flex-wrap: wrap; }
  .wr .gg-i { display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .wr .gg-r { position: relative; overflow: hidden; }
  .wr .gg-r svg { display: block; }
  .wr .gg-c { position: absolute; left: 0; right: 0; top: 0; height: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; padding-top: 6%; }
  .wr .gg-c b { font-size: calc(30px * var(--s, 1)); font-weight: 600; color: var(--title); letter-spacing: -.02em; line-height: 1; }
  .wr .gg-c b small { font-size: .5em; font-weight: 500; color: var(--muted); margin-left: 2px; }
  .wr .gg-c span { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); margin-top: 4px; }
  .wr .gg-i p { font-size: calc(16px * var(--s, 1)); font-weight: 500; color: var(--title); text-align: center; }
  .wr .hm { display: grid; gap: 3px; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); }
  .wr .hm .c { display: grid; place-items: center; border-radius: 4px; min-height: 18px; }
  .wr .hm.cal .c.out { opacity: .35; }
  .wr .hm.mx { gap: 4px; } .wr .hm.mx .c { min-height: calc(30px * var(--s, 1)); }
  .wr .hm.mx .c b { font-size: calc(13px * var(--s, 1)); font-weight: 500; }
  .wr .hm .hx { text-align: center; padding-bottom: 2px; } .wr .hm .hy { padding-right: 8px; align-self: center; white-space: nowrap; }
  .wr .hm.cal span { align-self: center; white-space: nowrap; }
  .wr .hm-k { display: flex; align-items: center; gap: 4px; justify-content: flex-end; font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--muted); }
  .wr .hm-k i { width: 12px; height: 12px; border-radius: 3px; } .wr .hm-k span { margin: 0 4px; }
  .wr .fn { display: flex; flex-direction: column; gap: 4px; }
  .wr .fn-r { display: grid; grid-template-columns: minmax(110px, 26%) 1fr 96px; gap: 14px; align-items: center; font-size: calc(16px * var(--s, 1)); }
  .wr .fn-l { color: var(--title); font-weight: 500; }
  .wr .fn-t { display: flex; justify-content: center; height: calc(30px * var(--s, 1)); }
  .wr .fn-t i { display: block; height: 100%; border-radius: 6px; }
  .wr .fn-v { font: 500 calc(14px * var(--s, 1)) var(--mono), monospace; color: var(--title); text-align: right; }
  .wr .fn-v small { margin-left: 6px; color: var(--muted); font-weight: 400; }
  .wr .fn-s { font: calc(13px * var(--s, 1)) var(--mono), monospace; color: var(--faint); text-align: center; padding-left: calc(26% + 14px); padding-right: 110px; }
`;

  const THAI = /[฀-๿]/;
  const lab = (t) => `<p class="label${THAI.test(t) ? " th" : ""}">◇ ${esc(t)}</p>`;

  function make(theme0) {
    const specs = [];   // chart blocks, drawn after the report is in the page
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
      out.push(...typeVars(style));
      return out.length ? ` style="${out.join(";")}"` : "";
    };
    // a block's own parts (style.<part>) → CSS variables the block's CSS falls back from
    const parts = (style, map) => {
      if (!style) return "";
      const out = Object.entries(map).filter(([k]) => style[k] != null && typeof style[k] !== "object").map(([k, v]) => `${v}:${col(style[k], style)}`);
      return out.join(";");
    };
    const withParts = (style, map) => { const a = vars(style), b = parts(style, map); if (!b) return a; return a ? a.replace(/"$/, `;${b}"`) : ` style="${b}"`; };
    // fonts, sizes and weights (report theme or block style)
    function typeVars(o) {
      const out = [];
      if (o.font) out.push(`--font:'${String(o.font).replace(/['";]/g, "")}'`);
      if (o.mono) out.push(`--mono:'${String(o.mono).replace(/['";]/g, "")}'`);
      if (+o.scale > 0) out.push(`--s:${+o.scale}`);
      for (const [k, v] of Object.entries(o.sizes ?? {})) if (+v > 0) out.push(`--fs-${k}:${+v}px`);
      for (const [k, v] of Object.entries(o.weights ?? {})) if (+v > 0) out.push(`--fw-${k}:${+v}`);
      return out;
    }
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
        return `<div class="stats"${withParts(b.style, { bar: "--sbar", value: "--sval" })}>${(b.items ?? []).map((it) => {
          const c = col(it.color, b.style);
          const d = it.delta ? chip(it.delta, it.kind ?? (/^[↓-]/.test(it.delta) ? "ok" : /^[↑+]/.test(it.delta) ? "ok" : "neutral"), b.style) : "";
          const bar = typeof it.bar === "number" ? `<div class="stat-bar"><i style="width:${Math.max(2, Math.min(100, it.bar * 100))}%"></i></div>` : "";
          const own = [c ? `--vcol:${c}` : "", it.bg ? `background:${col(it.bg, b.style)}` : "", it.border ? `border-color:${col(it.border, b.style)}` : "", it.ink ? `--title:${col(it.ink, b.style)};--muted:${col(it.ink, b.style)};--text:${col(it.ink, b.style)}` : ""].filter(Boolean).join(";");
          return `<div class="card"${own ? ` style="${own}"` : ""}>${lab(it.label ?? "")}<div class="stat-v"><b>${esc(it.value)}</b>${it.unit ? `<span>${esc(it.unit)}</span>` : ""}</div>${bar}${d || it.note ? `<div class="stat-f">${d}${it.note ? `<span>${esc(it.note)}</span>` : ""}</div>` : ""}</div>`;
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
        const kind = b.kind ?? (b.items && !b.series ? "hbars" : "line");
        specs.push(b);
        const multi = ["line", "bars", "stacked", "area"].includes(kind) && (b.series ?? []).length > 1;
        const pal = (b.style?.series ?? theme0.series ?? []).map((c) => col(c, b.style));
        const legend = multi ? `<div class="legend">${b.series.map((s0, i) => `<span><i style="background:${col(s0.color, b.style) ?? pal[i % Math.max(1, pal.length)]}"></i>${esc(s0.name ?? "")}</span>`).join("")}</div>` : b.badge ? chipOf(b.badge, b.style) : "";
        return `<div class="card"${vars(b.style)}>${head(b, legend)}<div class="chart-host" data-c="${specs.length - 1}"></div>
          ${b.note ? `<div class="note"><span class="label">${esc(b.noteLabel ?? "note")}</span><span>${esc(b.note)}</span></div>` : ""}</div>`;
      },
      board(b) {
        const cols = b.columns ?? [];
        return `<div class="card"${vars(b.style)}>${head(b)}<div class="cols">${cols.map((c, i) => {
          const st = b.style?.columns?.[i] ?? {};
          const dot = col(st.dot ?? c.color, b.style) ?? KIND(c.kind ?? "neutral", b.style).dot;
          return `<div class="col" style="--dot:${dot}${st.bg ? `;--colbg:${col(st.bg, b.style)}` : ""}"><div class="col-h"><i></i>${esc(c.title)}<small>${(c.cards ?? []).length}</small></div>${(c.cards ?? []).map((k) => `<div class="bcard"${k.bg || k.border || k.ink ? ` style="${[k.bg ? `background:${col(k.bg, b.style)}` : "", k.border ? `border-color:${col(k.border, b.style)}` : "", k.ink ? `--title:${col(k.ink, b.style)};--muted:${col(k.ink, b.style)}` : ""].filter(Boolean).join(";")}"` : ""}>${k.status ? chipOf(k.status, b.style) : ""}<b>${esc(k.title)}</b>${k.body ? `<span>${esc(k.body)}</span>` : ""}</div>`).join("")}</div>`;
        }).join("")}</div></div>`;
      },
      table(b) {
        const preset = b.preset ?? "basic";
        const rows = b.rows ?? [];
        const cols = (b.columns ?? []).map((c, i) => {
          const o = typeof c === "string" ? { name: c } : { ...c };
          if (!o.kind) {
            const sample = rows.map((r) => (Array.isArray(r) ? r : r?.cells)).find((r) => Array.isArray(r) && r[i] != null && r[i] !== "")?.[i];
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
        const body = rows.map((r) => {
          if (typeof r === "string") return `<tr class="group"><td colspan="${cols.length}">${esc(r)}</td></tr>`;
          const cells = Array.isArray(r) ? r : r.cells ?? [];
          const st = Array.isArray(r) ? "" : [r.bg ? `--row:${col(r.bg, b.style)}` : "", r.ink ? `--text:${col(r.ink, b.style)};--title:${col(r.ink, b.style)}` : "", r.bold ? "font-weight:600" : ""].filter(Boolean).join(";");
          return `<tr class="r"${st ? ` style="${st}"` : ""}>${cols.map((c, i) => cell(c, cells[i], i)).join("")}</tr>`;
        }).join("");
        const total = b.total ? `<tr class="total">${cols.map((c, i) => `<td class="${c.kind === "num" ? "num" : ""}">${esc(b.total[i] ?? "")}</td>`).join("")}</tr>` : "";
        return `<div class="card flush"${withParts(b.style, { head: "--th-bg", headText: "--th-tx", zebra: "--zebra", rule: "--rule", total: "--total", row: "--row" })}>${head(b)}<table class="${cls}"><thead><tr>${cols.map((c) => `<th class="${c.kind === "num" ? "num" : ""}">${esc(c.name)}</th>`).join("")}</tr></thead><tbody>${body}${total}</tbody></table></div>`;
      },
      timeline(b) {
        const it = b.items ?? [];
        const dot = (k) => { const d = b.style?.dot; return d == null ? null : typeof d === "object" ? col(d[k], b.style) : col(d, b.style); };
        return `<div class="card"${withParts(b.style, { line: "--tl-line", time: "--tl-time" })}>${head(b)}<div class="tl">${it.map((x, i) => `<div class="tl-i"><span class="tm">${esc(x.time ?? "")}</span><span class="rail"><i style="background:${col(x.color, b.style) ?? dot(x.kind ?? "info") ?? KIND(x.kind ?? "info", b.style).dot}"></i>${i < it.length - 1 ? "<s></s>" : ""}</span><p>${esc(x.text)}</p></div>`).join("")}</div></div>`;
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
        return `<div class="card"${withParts(b.style, { step: "--fl-bg", border: "--fl-bd", actor: "--fl-actor" })}>${head(b)}<div class="fl">${s.map((x, i) => `${i ? arrow : ""}<div class="fl-s" style="${[x.kind ? `border-color:${KIND(x.kind, b.style).line}` : "", x.bg ? `background:${col(x.bg, b.style)}` : "", x.border ? `border-color:${col(x.border, b.style)}` : "", x.ink ? `--title:${col(x.ink, b.style)};--fl-actor:${col(x.ink, b.style)}` : ""].filter(Boolean).join(";")}">${x.actor ? lab(x.actor) : ""}<b>${esc(x.text)}</b></div>`).join("")}</div></div>`;
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
        const dotOf = (s) => KIND(K[s]?.[1] ?? "neutral", b.style).dot;
        if (b.view === "board") {
          const cols = ["todo", "doing", "review", "done"].filter((s) => items.some((t) => t.status === s));
          return `<div class="card flush"${vars(b.style)}>${head({ ...b, title }, badges)}<div class="tb-cols">${cols.map((s) => `<div class="tb-col"><h4><i style="background:${dotOf(s)}"></i>${K[s][0]}<small>${items.filter((t) => t.status === s).length}</small></h4>${items.filter((t) => t.status === s).map((t) => `<div class="tb-card">${esc(t.title)}${where(t) ? `<small>${esc(where(t))}</small>` : ""}</div>`).join("")}</div>`).join("") || '<p class="cap" style="padding:0 0 4px">No tasks match.</p>'}</div></div>`;
        }
        if (b.view === "list") {
          return `<div class="card flush"${vars(b.style)}>${head({ ...b, title }, badges)}<div class="tb-list">${items.map((t) => `<div class="tb-li"><i style="background:${dotOf(t.status)}"></i><span>${esc(t.title)}</span><small>${esc(K[t.status]?.[0] ?? t.status)}</small></div>`).join("") || '<p class="cap">No tasks match.</p>'}</div></div>`;
        }
        return `<div class="card flush"${vars(b.style)}>${head({ ...b, title }, badges)}<table class="tbl compact"><thead><tr><th>Task</th><th>Status</th><th>Where</th></tr></thead><tbody>${items.map((t) => `<tr class="r"><td class="first">${esc(t.title)}</td><td style="width:130px">${chip(...(K[t.status] ?? [t.status, "neutral"]), b.style)}</td><td class="link">${esc(where(t))}</td></tr>`).join("") || `<tr><td colspan="3" style="color:var(--muted)">No tasks match.</td></tr>`}</tbody></table></div>`;
      },
      divider() { return `<div class="rule"></div>`; },
    };
    return { B, T, col, KIND, specs };
  }

  // ---------------------------------------------------------------- charts
  // A chart block leaves a placeholder; once the report is in the page, drawCharts measures each one and draws
  // it at its real pixel size, so labels never stretch with the chart.
  const CHART_KINDS = ["line", "bars", "hbars", "stacked", "area", "donut", "pie", "scatter", "gauge", "heatmap", "funnel"];
  const nice = (v) => { const e = Math.pow(10, Math.floor(Math.log10(Math.abs(v) || 1))), f = v / e; return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * e; };
  function ticks(lo, hi, n = 4) {
    if (hi === lo) hi = lo + 1;
    const step = nice((hi - lo) / n);
    const a = Math.floor(lo / step) * step, b = Math.ceil(hi / step) * step;
    const out = []; for (let v = a; v <= b + step / 2; v += step) out.push(+v.toFixed(10));
    return out;
  }
  const fmtNum = (v, unit) => {
    if (v == null || Number.isNaN(+v)) return "";
    const n = +v, a = Math.abs(n);
    const s = a >= 1000 ? n.toLocaleString("en-US", { maximumFractionDigits: 1 }) : a >= 100 ? String(Math.round(n * 10) / 10) : String(Math.round(n * 100) / 100);
    return unit ? `${s}${/^[%°]/.test(unit) ? "" : " "}${unit}` : s;
  };
  let measurer;
  const textW = (s, px = 11, font = "IBM Plex Mono") => { measurer ??= document.createElement("canvas").getContext("2d"); measurer.font = `${px}px "${font}", monospace`; return measurer.measureText(String(s)).width; };

  function drawCharts(root, specs, T, col, KIND) {
    const css = getComputedStyle(root);
    const v = (k) => css.getPropertyValue(`--${k}`).trim();
    const scale = parseFloat(css.getPropertyValue("--s")) || 1;
    const fs = 13 * scale;
    root.querySelectorAll(".chart-host").forEach((host) => {
      const b = specs[+host.dataset.c];
      try { host.innerHTML = draw(b, host.clientWidth || 560); } catch (e) { host.innerHTML = `<p class="cap">Couldn't draw this chart: ${esc(e.message)}</p>`; }
    });

    function draw(b, W) {
      const kind = b.kind ?? (b.items && !b.series ? "hbars" : "line");
      const pal = (b.style?.series ?? T("series") ?? []).map((c) => col(c, b.style));
      const colorAt = (i, c) => col(c, b.style) ?? pal[i % Math.max(1, pal.length)] ?? T("accent");
      const unit = b.unit ?? "";
      const mono = v("mono").replace(/"/g, "") || "IBM Plex Mono";
      const tx = (x, y, s, o = {}) => `<text x="${x.toFixed(1)}" y="${y.toFixed(1)}" text-anchor="${o.a ?? "middle"}"${o.cls ? ` class="${o.cls}"` : ""}${o.fill ? ` style="fill:${o.fill}"` : ""}${o.rot ? ` transform="rotate(${o.rot} ${x.toFixed(1)} ${y.toFixed(1)})"` : ""}>${esc(s)}</text>`;
      if (["line", "bars", "stacked", "area"].includes(kind)) return cartesian(kind);
      if (kind === "hbars") return hbars();
      if (kind === "donut" || kind === "pie") return donut(kind === "pie" ? 0 : b.hole ?? 0.62);
      if (kind === "scatter") return scatter();
      if (kind === "gauge") return gauge();
      if (kind === "heatmap") return heatmap();
      if (kind === "funnel") return funnel();
      throw new Error(`unknown chart kind "${kind}" — ${CHART_KINDS.join(", ")}`);

      function cartesian(kind) {
        const ss = (b.series ?? []).map((s, i) => ({ ...s, values: (s.values ?? []).map((x) => (x == null ? null : +x)), color: colorAt(i, s.color) }));
        const n = Math.max(1, ...ss.map((s) => s.values.length), (b.x ?? []).length);
        const stacked = kind === "stacked" || (kind === "area" && b.stacked !== false && ss.length > 1);
        const sums = Array.from({ length: n }, (_, i) => ss.reduce((a, s) => a + (s.values[i] ?? 0), 0));
        const all = stacked ? sums : ss.flatMap((s) => s.values.filter((x) => x != null));
        const tk = ticks(b.min ?? Math.min(0, ...all), b.max ?? Math.max(...all, 0));
        const lo = tk[0], hi = tk[tk.length - 1];
        const H = (b.height ?? 240) * scale;
        const yAxis = b.yAxis !== false;
        const left = (yAxis ? Math.max(...tk.map((t) => textW(fmtNum(t), fs, mono))) + 10 : 4) + (b.yTitle ? 18 : 0);
        const valueTop = 18 * scale + (b.marks?.length ? 16 * scale : 0);
        const bottom = (b.x?.length ? 22 * scale : 6) + (b.xTitle ? 18 * scale : 0);
        const right = 12;
        const pw = W - left - right, ph = H - valueTop - bottom;
        const y = (val) => valueTop + (1 - (val - lo) / (hi - lo || 1)) * ph;
        const bar = kind === "bars" || kind === "stacked";
        const slot = pw / n;
        const xAt = (i) => left + (bar ? slot * (i + 0.5) : n === 1 ? pw / 2 : (i * pw) / (n - 1));
        const grid = col(b.style?.grid ?? "grid", b.style);
        let g = "";
        for (const t of tk) {
          g += `<line x1="${left}" x2="${W - right}" y1="${y(t).toFixed(1)}" y2="${y(t).toFixed(1)}" stroke="${grid}"${t === 0 && lo < 0 ? ' stroke-width="1.5"' : ""}/>`;
          if (yAxis) g += tx(left - 8, y(t) + fs * 0.35, fmtNum(t), { a: "end" });
        }
        if (b.yTitle) g += tx(fs, valueTop + ph / 2, b.yTitle, { rot: -90 });
        // x labels: as many as fit
        if (b.x?.length) {
          const maxW = Math.max(...b.x.map((s) => textW(s, fs, mono))) + 8;
          const every = Math.max(1, Math.ceil(maxW / (pw / Math.max(1, n - (bar ? 0 : 1)) || maxW)));
          b.x.forEach((s, i) => { if (i % every === 0 || i === n - 1) g += tx(xAt(i), valueTop + ph + 16 * scale, s, { a: !bar && i === 0 ? "start" : !bar && i === n - 1 ? "end" : "middle" }); });
        }
        if (b.xTitle) g += tx(left + pw / 2, H - 4, b.xTitle);
        const show = b.labels ?? (bar ? (n * (kind === "stacked" ? 1 : ss.length) <= 24 ? "all" : "none") : "last");
        const label = (x, yy, val, fill) => tx(x, yy, fmtNum(val, unit), { cls: "v", fill });
        const fade = (i) => (b.highlight != null && i !== b.highlight ? ' fill-opacity=".35"' : "");
        if (kind === "bars") {
          const bw = Math.min(56 * scale, (slot * 0.72) / Math.max(1, ss.length));
          ss.forEach((s, si) => s.values.forEach((val, i) => {
            if (val == null) return;
            const x = left + slot * i + (slot - bw * ss.length) / 2 + si * bw, top = y(val), base = y(Math.max(lo, 0));
            g += `<rect x="${(x + 1.5).toFixed(1)}" y="${Math.min(top, base).toFixed(1)}" width="${Math.max(1, bw - 3).toFixed(1)}" height="${Math.max(1, Math.abs(base - top)).toFixed(1)}" rx="${Math.min(5, bw / 4)}" fill="${col(s.colors?.[i], b.style) ?? s.color}"${fade(i)}/>`;
            if (show === "all" || (show === "max" && val === Math.max(...s.values)) || (show === "last" && i === s.values.length - 1)) g += label(x + bw / 2, Math.min(top, base) - 6, val);
          }));
        } else if (kind === "stacked") {
          const bw = Math.min(64 * scale, slot * 0.62);
          for (let i = 0; i < n; i++) {
            let acc = 0;
            ss.forEach((s, si) => {
              const val = s.values[i] ?? 0; if (!val) return;
              const y0 = y(acc), y1 = y(acc + val); acc += val;
              const top = si === ss.length - 1 || ss.slice(si + 1).every((t) => !(t.values[i] ?? 0));
              g += `<rect x="${(xAt(i) - bw / 2).toFixed(1)}" y="${y1.toFixed(1)}" width="${bw.toFixed(1)}" height="${Math.max(1, y0 - y1 - 1.5).toFixed(1)}" rx="${top ? 4 : 0}" fill="${s.color}"${fade(i)}/>`;
              if (show === "all" && y0 - y1 > fs + 4) g += tx(xAt(i), (y0 + y1) / 2 + fs * 0.35, fmtNum(val), { fill: "#fff", cls: "v" });
            });
            if (show !== "none") g += label(xAt(i), y(sums[i]) - 6, sums[i]);
          }
        } else {
          // line / area
          let base = Array(n).fill(0);
          // value labels already placed at each x: a second series' label moves below its point instead of on top
          const placed = {};
          ss.forEach((s, si) => {
            const pts = s.values.map((val, i) => (val == null ? null : [xAt(i), y(stacked ? base[i] + val : val)]));
            const d = pts.map((p, i) => (p ? `${i && pts[i - 1] ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}` : "")).join(" ");
            if (kind === "area") {
              const lower = stacked ? base.map((v0, i) => [xAt(i), y(v0)]).reverse() : [[xAt(n - 1), y(Math.max(lo, 0))], [xAt(0), y(Math.max(lo, 0))]];
              g += `<path d="${d} ${lower.map((p) => `L${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(" ")} Z" fill="${s.color}" fill-opacity="${stacked ? 0.78 : 0.18}"/>`;
            } else if (si === 0 && ss.length === 1 && (b.style?.area ?? "on") !== "none") {
              g += `<path d="${d} L${xAt(s.values.length - 1).toFixed(1)} ${y(Math.max(lo, 0)).toFixed(1)} L${xAt(0).toFixed(1)} ${y(Math.max(lo, 0)).toFixed(1)} Z" fill="${s.color}" fill-opacity=".08"/>`;
            }
            g += `<path d="${d}" fill="none" stroke="${s.color}" stroke-width="${kind === "area" && stacked ? 1.5 : 2.5}" stroke-linejoin="round" stroke-linecap="round"/>`;
            const last = s.values.length - 1, mx = Math.max(...s.values.filter((x) => x != null));
            pts.forEach((p, i) => {
              if (!p) return;
              if (kind === "line" && b.points !== false) g += `<circle cx="${p[0].toFixed(1)}" cy="${p[1].toFixed(1)}" r="${i === last ? 5 : 3.5}" fill="${i === last ? s.color : T("card")}" stroke="${s.color}" stroke-width="2"/>`;
              const want = (!stacked || si === ss.length - 1 || show === "all") && (show === "all" || (show === "last" && i === last) || (show === "max" && s.values[i] === mx));
              if (want) {
                let ly = p[1] - 10;
                const near = (placed[i] ??= []);
                if (near.some((o) => Math.abs(o - ly) < fs * 1.3)) ly = p[1] + fs + 8;
                if (near.some((o) => Math.abs(o - ly) < fs * 1.3)) ly = Math.max(...near) + fs * 1.3;
                near.push(ly);
                g += label(p[0] + (i === last && show !== "all" ? -2 : 0), ly, stacked ? base[i] + s.values[i] : s.values[i], s.color);
              }
            });
            if (stacked) base = base.map((v0, i) => v0 + (s.values[i] ?? 0));
          });
        }
        // marks: a dashed line with a note at an x
        for (const m of b.marks ?? []) {
          const i = typeof m.x === "number" ? m.x : (b.x ?? []).indexOf(m.x);
          if (i < 0) continue;
          const x = xAt(i), c = col(m.color, b.style) ?? T("muted");
          g += `<line x1="${x.toFixed(1)}" x2="${x.toFixed(1)}" y1="${valueTop - 6}" y2="${valueTop + ph}" stroke="${c}" stroke-dasharray="3 3"/>` + tx(x, valueTop - 10, m.text ?? "", { fill: c, cls: "m" });
        }
        return `<svg class="chart" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">${g}</svg>`;
      }

      function hbars() {
        let items = (b.items ?? []).map((it, i) => ({ ...it, value: +it.value || 0, i }));
        if (b.sort !== false) items.sort((a, c) => c.value - a.value);
        const max = b.max ?? Math.max(...items.map((x) => x.value), 1);
        return `<div class="hb">${items.map((it, k) => `<div class="hb-r"${b.highlight != null && k !== b.highlight ? ' style="opacity:.45"' : ""}><span class="hb-l">${b.rank !== false ? `<em>${k + 1}</em>` : ""}${esc(it.label)}</span><span class="hb-t"><i style="width:${Math.max(0.5, (it.value / max) * 100)}%;background:${col(it.color, b.style) ?? (k === 0 || b.highlight != null ? T("accent") : `color-mix(in srgb, ${T("accent")} 50%, ${T("card")})`)}"></i></span><span class="hb-v">${esc(it.text ?? fmtNum(it.value, unit))}</span></div>`).join("")}</div>`;
      }

      function donut(hole) {
        const items = (b.items ?? []).map((it, i) => ({ ...it, value: Math.max(0, +it.value || 0), color: colorAt(i, it.color) }));
        const total = items.reduce((a, x) => a + x.value, 0) || 1;
        const D = Math.min(220 * scale, W * 0.42), R = D / 2, r = R * hole;
        let a0 = -Math.PI / 2, g = "";
        const pt = (rad, a) => [R + rad * Math.cos(a), R + rad * Math.sin(a)];
        for (const it of items) {
          if (!it.value) continue;
          const a1 = a0 + (it.value / total) * Math.PI * 2 - (items.length > 1 ? 0.012 : 0);
          const big = a1 - a0 > Math.PI ? 1 : 0;
          if (it.value === total) g += hole ? `<circle cx="${R}" cy="${R}" r="${(R + r) / 2}" fill="none" stroke="${it.color}" stroke-width="${R - r}"/>` : `<circle cx="${R}" cy="${R}" r="${R}" fill="${it.color}"/>`;
          else {
            const [x0, y0] = pt(R, a0), [x1, y1] = pt(R, a1), [x2, y2] = pt(r, a1), [x3, y3] = pt(r, a0);
            g += `<path d="M${x0} ${y0} A${R} ${R} 0 ${big} 1 ${x1} ${y1} ${hole ? `L${x2} ${y2} A${r} ${r} 0 ${big} 0 ${x3} ${y3}` : `L${R} ${R}`} Z" fill="${it.color}"/>`;
          }
          a0 = a1 + (items.length > 1 ? 0.012 : 0);
        }
        const c = b.center ?? (hole ? { value: fmtNum(total, unit), label: "total" } : null);
        const centre = c && hole ? `<div class="dn-c"><b>${esc(c.value)}</b>${c.label ? `<span>${esc(c.label)}</span>` : ""}</div>` : "";
        return `<div class="dn"><div class="dn-g" style="width:${D}px;height:${D}px"><svg width="${D}" height="${D}" viewBox="0 0 ${D} ${D}">${g}</svg>${centre}</div>
          <div class="dn-l">${items.map((it) => `<div class="dn-i"><i style="background:${it.color}"></i><span>${esc(it.label)}</span><b>${esc(it.text ?? fmtNum(it.value, unit))}</b><small>${Math.round((it.value / total) * 100)}%</small></div>`).join("")}</div></div>`;
      }

      function scatter() {
        const groups = b.series?.length ? b.series.map((s, i) => ({ name: s.name, color: colorAt(i, s.color), points: s.points ?? [] })) : [{ color: colorAt(0), points: b.points ?? [] }];
        const pts = groups.flatMap((g0) => g0.points);
        const tx0 = ticks(b.xMin ?? Math.min(...pts.map((p) => +p.x)), b.xMax ?? Math.max(...pts.map((p) => +p.x)));
        const ty = ticks(b.min ?? Math.min(0, ...pts.map((p) => +p.y)), b.max ?? Math.max(...pts.map((p) => +p.y)));
        const H = (b.height ?? 260) * scale;
        const left = Math.max(...ty.map((t) => textW(fmtNum(t), fs, mono))) + 10 + (b.yTitle ? 18 : 0), top = 14, bottom = 22 * scale + (b.xTitle ? 18 * scale : 0), right = 14;
        const pw = W - left - right, ph = H - top - bottom;
        const X = (x) => left + ((x - tx0[0]) / (tx0.at(-1) - tx0[0] || 1)) * pw, Y = (y) => top + (1 - (y - ty[0]) / (ty.at(-1) - ty[0] || 1)) * ph;
        const grid = col(b.style?.grid ?? "grid", b.style);
        let g = ty.map((t) => `<line x1="${left}" x2="${W - right}" y1="${Y(t).toFixed(1)}" y2="${Y(t).toFixed(1)}" stroke="${grid}"/>` + tx(left - 8, Y(t) + fs * 0.35, fmtNum(t), { a: "end" })).join("");
        g += tx0.map((t) => tx(X(t), top + ph + 16 * scale, fmtNum(t))).join("");
        if (b.xTitle) g += tx(left + pw / 2, H - 4, b.xTitle);
        if (b.yTitle) g += tx(fs, top + ph / 2, b.yTitle, { rot: -90 });
        for (const gr of groups) for (const p of gr.points) {
          const c = col(p.color, b.style) ?? gr.color, rad = (p.size ?? 5) * scale;
          g += `<circle cx="${X(+p.x).toFixed(1)}" cy="${Y(+p.y).toFixed(1)}" r="${rad}" fill="${c}" fill-opacity=".75" stroke="${c}"/>`;
          if (p.label) g += tx(X(+p.x) + rad + 4, Y(+p.y) + fs * 0.35, p.label, { a: "start", cls: "v" });
        }
        return `<svg class="chart" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">${g}</svg>`;
      }

      function gauge() {
        const items = b.items ?? [{ label: b.label, value: b.value, max: b.max, unit: b.unit, color: b.color, kind: b.gaugeKind }];
        const D = Math.min(170 * scale, (W - 16 * (items.length - 1)) / items.length);
        const sw = Math.max(8, D * 0.09), R = D / 2 - sw / 2;
        const arc = (f) => { const a0 = Math.PI * 0.75, a1 = a0 + Math.PI * 1.5 * Math.max(0.0001, Math.min(1, f)); const p = (a) => [D / 2 + R * Math.cos(a), D / 2 + R * Math.sin(a)]; const [x0, y0] = p(a0), [x1, y1] = p(a1); return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${R} ${R} 0 ${a1 - a0 > Math.PI ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`; };
        return `<div class="gg">${items.map((it, i) => {
          const max = +(it.max ?? 100) || 100, f = (+it.value || 0) / max;
          const c = col(it.color, b.style) ?? (it.kind ? KIND(it.kind, b.style).dot : colorAt(i));
          return `<div class="gg-i"><div class="gg-r" style="width:${D}px;height:${D * 0.86}px"><svg width="${D}" height="${D}" viewBox="0 0 ${D} ${D}"><path d="${arc(1)}" fill="none" stroke="${T("soft")}" stroke-width="${sw}" stroke-linecap="round"/><path d="${arc(f)}" fill="none" stroke="${c}" stroke-width="${sw}" stroke-linecap="round"/></svg>
            <div class="gg-c"><b>${esc(it.text ?? fmtNum(it.value))}<small>${esc(it.unit ?? (max === 100 && !it.unit ? "%" : ""))}</small></b>${it.max != null && max !== 100 ? `<span>of ${esc(fmtNum(max))}</span>` : ""}</div></div>${it.label ? `<p>${esc(it.label)}</p>` : ""}</div>`;
        }).join("")}</div>`;
      }

      function heatmap() {
        const hot = col(b.style?.hot ?? "accent", b.style), cold = col(b.style?.cold ?? "soft", b.style);
        const shade = (val, max) => (val == null ? cold : `color-mix(in srgb, ${hot} ${Math.round(12 + 88 * Math.min(1, val / (max || 1)))}%, ${T("card")})`);
        const legend = `<div class="hm-k"><span>less</span>${[0.15, 0.4, 0.65, 0.9].map((f) => `<i style="background:${shade(f, 1)}"></i>`).join("")}<span>more</span></div>`;
        if (b.days?.length) {
          // calendar: a column per week, a row per weekday (Mon first)
          const days = b.days.map((d) => ({ ...d, t: new Date(`${d.date}T00:00:00`) })).filter((d) => !Number.isNaN(+d.t)).sort((a, c) => a.t - c.t);
          const start = new Date(days[0].t); start.setDate(start.getDate() - ((start.getDay() + 6) % 7));
          const weeks = Math.ceil(((days.at(-1).t - start) / 864e5 + 1) / 7);
          const max = Math.max(...days.map((d) => +d.value || 0));
          const cell = Math.max(10, Math.min(40 * scale, (W - 44) / weeks - 4));
          const map = new Map(days.map((d) => [d.date, d]));
          let grid = "";
          for (let w = 0; w < weeks; w++) for (let dd = 0; dd < 7; dd++) {
            const t = new Date(start); t.setDate(t.getDate() + w * 7 + dd);
            const key = `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, "0")}-${String(t.getDate()).padStart(2, "0")}`;
            const d = map.get(key);
            grid += `<span class="c${d ? "" : " out"}" style="grid-column:${w + 2};grid-row:${dd + 2};background:${d ? shade(+d.value || 0, max) : cold}"${d ? ` title="${esc(key)}: ${esc(fmtNum(d.value, unit))}"` : ""}></span>`;
          }
          // a month's name over the first week that starts in it (never two names closer than 3 weeks)
          const months = []; let lastAt = -9;
          for (let w = 0; w < weeks; w++) { const t = new Date(start); t.setDate(t.getDate() + w * 7 + 6); if ((w === 0 || t.getDate() <= 7) && w - lastAt >= 3) { lastAt = w; months.push(`<span style="grid-column:${w + 2} / span 3;grid-row:1">${t.toLocaleDateString("en-GB", { month: "short" })}</span>`); } }
          const wd = ["Mon", "", "Wed", "", "Fri", "", ""].map((s, i) => (s ? `<span style="grid-column:1;grid-row:${i + 2}">${s}</span>` : "")).join("");
          return `<div class="hm cal" style="grid-template-columns:40px repeat(${weeks}, ${cell}px);grid-template-rows:auto repeat(7, ${Math.min(cell, 30 * scale)}px)">${months.join("")}${wd}${grid}</div>${legend}`;
        }
        const xs = b.x ?? [], ys = b.y ?? [], vals = b.values ?? [];
        const max = Math.max(...vals.flat().map((x) => +x || 0));
        const show = b.labels === "all";
        return `<div class="hm mx" style="grid-template-columns:auto repeat(${xs.length}, 1fr)"><span></span>${xs.map((x) => `<span class="hx">${esc(x)}</span>`).join("")}${ys.map((yl, r) => `<span class="hy">${esc(yl)}</span>${xs.map((_, c) => { const val = vals[r]?.[c]; return `<span class="c" style="background:${shade(val == null ? null : +val, max)}">${show && val != null ? `<b style="color:${+val / max > 0.55 ? "#fff" : T("text")}">${esc(fmtNum(val))}</b>` : ""}</span>`; }).join("")}`).join("")}</div>${legend}`;
      }

      function funnel() {
        const items = (b.items ?? []).map((it, i) => ({ ...it, value: +it.value || 0, color: colorAt(0, it.color) }));
        const first = items[0]?.value || 1;
        return `<div class="fn">${items.map((it, i) => `${i ? `<div class="fn-s">↓ ${Math.round((it.value / (items[i - 1].value || 1)) * 100)}% go on</div>` : ""}<div class="fn-r"><span class="fn-l">${esc(it.label)}</span><span class="fn-t"><i style="width:${Math.max(2, (it.value / first) * 100)}%;background:${col(it.color, b.style) ?? T("accent")};opacity:${1 - i * (0.55 / Math.max(1, items.length - 1))}"></i></span><span class="fn-v">${esc(it.text ?? fmtNum(it.value, unit))}<small>${Math.round((it.value / first) * 100)}%</small></span></div>`).join("")}</div>`;
      }
    }
  }

  // Draws `data` ({report, theme}) into `host`. ctx: { fileUrl(path) } for image paths in the project.
  function render(host, data, ctx = {}) {
    const theme = data.theme ?? {};
    const { B, T, col, KIND, specs } = make(theme);
    if (!document.getElementById("wr-css")) { const s = document.createElement("style"); s.id = "wr-css"; s.textContent = CSS; document.head.append(s); }
    const r = data.report ?? {};
    const root = document.createElement("div");
    root.className = "wr";
    root.style.cssText = ["page", "card", "line", "soft", "title", "text", "muted", "faint", "accent", "ok", "warn", "bad"].map((k) => `--${k}:${T(k)}`).join(";")
      + `;--radius:${theme.radius ?? 14}px;--font:"${theme.font ?? "IBM Plex Sans Thai"}";--mono:"${theme.mono ?? "IBM Plex Mono"}";width:${theme.width ?? 1440}px;`
      + ["--s:1", ...(function () { const o = []; if (+theme.scale > 0) o.push(`--s:${+theme.scale}`); for (const [k, v] of Object.entries(theme.sizes ?? {})) if (+v > 0) o.push(`--fs-${k}:${+v}px`); for (const [k, v] of Object.entries(theme.weights ?? {})) if (+v > 0) o.push(`--fw-${k}:${+v}`); return o; })()].join(";");
    if (theme.density === "compact") root.classList.add("compact");
    // fonts other than IBM Plex: load them from Google Fonts by name (the report and every block's style.font)
    const fams = new Set([theme.font, theme.mono]);
    for (const row of r.rows ?? []) for (const blk of Array.isArray(row) ? row : [row]) { fams.add(blk?.style?.font); fams.add(blk?.style?.mono); }
    const want = [...fams].filter((f) => f && !/^IBM Plex (Sans Thai|Mono)$/i.test(f) && !/^(system-ui|sans-serif|serif|monospace)$/i.test(f));
    root.fontsLoaded = Promise.all(want.map((f) => {
      const id = `wr-font-${f.replace(/\W+/g, "-")}`;
      if (document.getElementById(id)) return Promise.resolve();
      return new Promise((ok) => { const l = document.createElement("link"); l.id = id; l.rel = "stylesheet"; l.href = `https://fonts.googleapis.com/css2?family=${encodeURIComponent(f).replace(/%20/g, "+")}:wght@300;400;500;600;700&display=block`; l.onload = l.onerror = () => ok(); document.head.append(l); });
    })).then(() => document.fonts.ready);
    const one = (b) => { try { const out = (B[b?.type] ?? (() => `<div class="co" style="--cbg:#FBE7E4;--cbd:#F0C8C2;--ctx:#A42C22">Unknown block “${esc(b?.type)}”</div>`))(b ?? {}, ctx); return b?.style?.density === "compact" ? out.replace(/^<div class="card/, '<div class="card compact') : out; } catch (e) { return `<div class="co" style="--cbg:#FBE7E4;--cbd:#F0C8C2;--ctx:#A42C22">Couldn't draw a ${esc(b?.type)} block: ${esc(e.message)}</div>`; } };
    const rows = r.rows ?? [];
    // a report without a header block gets one from its title
    const hasHeader = rows.some((x) => (Array.isArray(x) ? x : [x]).some((b) => b?.type === "header"));
    let html = hasHeader ? "" : B.header({ title: r.title, subtitle: r.subtitle, eyebrow: r.eyebrow, status: r.status, meta: r.meta }) + `<div class="rule"></div>`;
    for (const row of rows) {
      if (Array.isArray(row)) html += `<div class="row">${row.map((b) => `<div style="flex:${b?.span ?? 1} 1 0;min-width:0;display:flex;flex-direction:column">${one(b).replace(/^<div class="card/, '<div class="card grow')}</div>`).join("")}</div>`;
      else html += one(row);
    }
    if (r.footer !== false) html += `<div class="rule"></div><div class="ft"><span>${esc(r.footer ?? `Made with wridge${data.name ? ` · docs/reports/${data.name}.json` : ""}`)}</span><span>${esc(r.updated ?? "")}</span></div>`;
    root.innerHTML = html;
    host.replaceChildren(root);
    drawCharts(root, specs, T, col, KIND);
    // labels are measured with the report's fonts: draw again once they are in
    root.fontsLoaded.then(() => drawCharts(root, specs, T, col, KIND));
    return root;
  }

  window.wrReport = { render, md, esc, CHART_KINDS };
})();
