// Generic mind-map renderer (static SVG, pan/zoom, lazy branches). Used by the code graph page.
// Tree node: { key, label, kind, badge?, children?: Node[], lazy?: () => Node[], data? }
// look(n) → { fill, stroke, text, badge, font, weight, h, pad, radius, pill, dot,
//             cls (classes on the node, for CSS), icon (an <svg> string, 14px tall; iconW its width), mono (label in IBM Plex Mono), minW }
// The badge sits at the right edge; a branch shows + / − inside its box. The edges from the root to the
// selected node get the class "on-path"; nodes and edges drawn for the first time get "mm-new" (entrance).
(() => {
  const NS = 'http://www.w3.org/2000/svg';
  const GAP_X = 56, GAP_Y = 12, ICON = 14, ICON_GAP = 8, TOGGLE = 10;
  const SANS = '"IBM Plex Sans Thai", system-ui, sans-serif', MONO = '"IBM Plex Mono", ui-monospace, Consolas, monospace';
  const measure = document.createElement('canvas').getContext('2d');
  const width = (text, size, weight, family = SANS) => {
    measure.font = `${weight} ${size}px ${family}`;
    return measure.measureText(text).width;
  };

  window.createMindMap = function createMindMap(svg, opts) {
    const root = document.createElementNS(NS, 'g');
    svg.appendChild(root);
    let tree = null, open = new Set(), view = { x: 0, y: 0, k: 1 };
    // highlight: (node) => level | undefined — the same symbol can appear in several places of the tree.
    let selected = null, highlight = null;
    const drawn = new Map();

    function el(name, attrs, parent) {
      const e = document.createElementNS(NS, name);
      for (const [k, v] of Object.entries(attrs)) if (v !== undefined && v !== '') e.setAttribute(k, v);
      parent.appendChild(e);
      return e;
    }

    const isOpen = (n) => n.depth === 0 || open.has(n.key);
    const kids = (n) => {
      if (n.lazy && !n.children) { n.children = n.lazy(); annotate(n.children, n.depth + 1, n); }
      return n.children ?? [];
    };
    const expandable = (n) => n.depth > 0 && (n.children ? n.children.length > 0 : Boolean(n.lazy));

    function annotate(list, depth, parent) {
      for (const n of list) {
        n.depth = depth; n.parent = parent;
        const st = opts.look(n);
        n.font = st.font ?? 13; n.weight = st.weight ?? 500; n.h = st.h ?? 34; n.pad = st.pad ?? 12;
        n.lw = width(n.label, n.font, n.weight, st.mono ? MONO : SANS);
        n.w = Math.max(st.minW ?? 0, (st.icon ? (st.iconW ?? ICON) + ICON_GAP : 0) + n.lw + n.pad * 2 + (n.badge ? width(n.badge, 11, 400, MONO) + 14 : 0) + (expandable(n) ? TOGGLE + 8 : 0));
        if (n.children) annotate(n.children, depth + 1, n);
      }
    }

    function span(n) {
      if (!isOpen(n)) return n.h;
      const c = kids(n);
      if (!c.length) return n.h;
      return Math.max(n.h, c.reduce((s, x) => s + span(x), 0) + GAP_Y * (c.length - 1));
    }
    function place(n, x, y, side) {
      n.x = x; n.y = y; n.side = side;
      if (!isOpen(n)) return;
      const c = kids(n);
      const total = c.reduce((s, x) => s + span(x), 0) + GAP_Y * (c.length - 1);
      let cy = y - total / 2;
      for (const ch of c) {
        const h = span(ch);
        place(ch, x + side * (n.w / 2 + GAP_X + ch.w / 2), cy + h / 2, side);
        cy += h + GAP_Y;
      }
    }
    function layout() {
      tree.x = 0; tree.y = 0;
      const left = [], right = [];
      let lh = 0, rh = 0;
      for (const c of kids(tree)) {
        // oneSide: everything grows to the right of the root, like a file tree
        if (opts.oneSide || rh <= lh) { right.push(c); rh += span(c) + GAP_Y * 2; } else { left.push(c); lh += span(c) + GAP_Y * 2; }
      }
      for (const [group, side] of [[right, 1], [left, -1]]) {
        const total = group.reduce((s, c) => s + span(c), 0) + GAP_Y * 2 * Math.max(0, group.length - 1);
        let cy = -total / 2;
        for (const c of group) {
          const h = span(c);
          place(c, side * (tree.w / 2 + GAP_X * 1.4 + c.w / 2), cy + h / 2, side);
          cy += h + GAP_Y * 2;
        }
      }
    }

    let seen = new Set(); // keys drawn by the previous render: only new ones play the entrance
    let edgeLayer = null;
    function render() {
      const before = seen;
      root.innerHTML = '';
      drawn.clear();
      const edges = el('g', {}, root), nodes = el('g', {}, root);
      edgeLayer = edges;
      const walk = (n) => {
        const fresh = !before.has(n.key);
        if (n.parent) {
          const p = n.parent, s = n.side;
          const x1 = p.x + s * p.w / 2, x2 = n.x - s * n.w / 2, mx = (x1 + x2) / 2;
          const e = opts.edge?.(n) ?? { color: '#CBD5E1', width: 1.25 };
          el('path', { class: `mm-edge${e.cls ? ` ${e.cls}` : ''}${fresh ? ' mm-new' : ''}`, 'data-key': n.key, d: `M${x1},${p.y} C${mx},${p.y} ${mx},${n.y} ${x2},${n.y}`, stroke: e.color, 'stroke-width': e.width, 'stroke-dasharray': e.dash }, edges);
        }
        drawNode(n, nodes, fresh);
        if (isOpen(n)) kids(n).forEach(walk);
      };
      walk(tree);
      seen = new Set(drawn.keys());
      apply();
      mark();
    }

    function drawNode(n, parent, fresh) {
      const st = opts.look(n);
      const g = el('g', { class: `mm-node${st.cls ? ` ${st.cls}` : ''}${fresh ? ' mm-new' : ''}`, transform: `translate(${n.x - n.w / 2},${n.y - n.h / 2})`, 'data-key': n.key }, parent);
      const box = el('g', { class: 'mm-in' }, g); // the entrance moves this inner group; the outer one keeps its place
      drawn.set(n.key, n);
      const rx = st.pill ? n.h / 2 : st.radius ?? 8;
      el('rect', { class: 'halo', x: -3, y: -3, width: n.w + 6, height: n.h + 6, rx: rx + 3 }, box);
      el('rect', { class: 'shadow', y: 1, width: n.w, height: n.h, rx, fill: 'none' }, box);
      el('rect', { class: 'body', width: n.w, height: n.h, rx, fill: st.fill, stroke: st.stroke ?? 'none', 'stroke-width': 1, 'stroke-dasharray': st.dash }, box);
      let x = n.pad;
      if (st.icon) {
        const ic = el('g', { class: 'mm-icon', transform: `translate(${x},${(n.h - ICON) / 2})` }, box);
        ic.innerHTML = st.icon;
        x += (st.iconW ?? ICON) + ICON_GAP;
      }
      const t = el('text', { class: st.mono ? 'label mono' : 'label', x, y: n.h / 2, fill: st.text, 'font-size': n.font, 'font-weight': n.weight, 'font-family': st.mono ? MONO : undefined }, box);
      t.textContent = n.label;
      const toggle = expandable(n);
      if (n.badge) {
        const b = el('text', { class: 'badge', x: n.w - n.pad - (toggle ? TOGGLE + 8 : 0), y: n.h / 2, 'text-anchor': 'end', fill: st.badge ?? '#94A3B8', 'font-size': 11, 'font-family': MONO }, box);
        b.textContent = n.badge;
      }
      if (st.dot) el('circle', { class: 'dot', cx: n.w - 4, cy: 4, r: 4, fill: st.dot, stroke: '#fff', 'stroke-width': 1.5 }, box);
      if (toggle) {
        const txt = el('text', { class: 'toggle', x: n.w - n.pad - TOGGLE / 2, y: n.h / 2, 'text-anchor': 'middle', fill: '#64748B', 'font-size': 13, 'font-weight': 500 }, box);
        txt.textContent = isOpen(n) ? '−' : '+';
      }
      g.addEventListener('click', (e) => {
        e.stopPropagation();
        if (expandable(n)) {
          if (open.has(n.key)) open.delete(n.key); else open.add(n.key);
          relayout();
        }
        opts.onSelect?.(n);
        ensureVisible(n);
      });
    }

    function mark() {
      // edges from the root down to the selected node, drawn last so they sit on top
      const path = new Set();
      for (let p = selected != null ? drawn.get(selected) : null; p; p = p.parent) path.add(p.key);
      const lit = [];
      root.querySelectorAll('.mm-edge').forEach((e) => {
        const on = path.has(e.getAttribute('data-key'));
        e.classList.toggle('on-path', on);
        if (on) lit.push(e);
      });
      if (edgeLayer) edgeLayer.append(...lit.reverse());
      root.querySelectorAll('.mm-node').forEach((g) => {
        const key = g.getAttribute('data-key');
        g.classList.toggle('selected', key === selected);
        g.classList.remove('hl-0', 'hl-1', 'hl-2', 'hl-3', 'dim', 'gs-hit', 'gs-current');
        if (!highlight) return;
        // null = leave as is, undefined = not affected, a number = impact level, a string = classes (search)
        const lvl = highlight(drawn.get(key));
        if (typeof lvl === 'string') { if (lvl) g.classList.add(...lvl.split(' ')); }
        else if (lvl !== null) g.classList.add(lvl === undefined ? 'dim' : `hl-${Math.min(lvl, 3)}`);
      });
    }

    function apply() {
      root.setAttribute('transform', `translate(${view.x},${view.y}) scale(${view.k})`);
      opts.onZoom?.(view.k);
    }

    function ensureVisible(n) {
      let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
      const walk = (m) => {
        minX = Math.min(minX, m.x - m.w / 2); maxX = Math.max(maxX, m.x + m.w / 2);
        minY = Math.min(minY, m.y - m.h / 2); maxY = Math.max(maxY, m.y + m.h / 2);
        if (isOpen(m)) kids(m).forEach(walk);
      };
      walk(n);
      const { top, bottom, right } = opts.freeArea();
      const sx = (x) => x * view.k + view.x, sy = (y) => y * view.k + view.y;
      if (sx(maxX) > right) view.x -= sx(maxX) - right;
      if (sx(minX) < 24) view.x += 24 - sx(minX);
      if (sy(maxY) > bottom) view.y -= sy(maxY) - bottom;
      if (sy(minY) < top) view.y += top - sy(minY);
      if (sy(n.y) < top || sy(n.y) > bottom) view.y = (top + bottom) / 2 - n.y * view.k;
      apply();
    }

    function relayout() { layout(); render(); }

    // all: show everything however small (the Fit button); otherwise stop at opts.minFit
    function fit(all = false) {
      const box = root.getBBox();
      if (!box.width) return;
      const { top, bottom, right } = opts.freeArea();
      const k = Math.min(opts.maxFit ?? 1.25, (right - 40) / box.width, Math.max(160, bottom - top) / box.height);
      view.k = Math.max(all ? 0.15 : opts.minFit ?? 0.15, k);
      if (k < view.k) {
        // too big to fit readably: keep the zoom, start at the root's side, centred on it
        view.x = Math.min((right + 24) / 2 - (box.x + box.width / 2) * view.k, 32 - box.x * view.k);
        view.y = top + (bottom - top) / 2 - tree.y * view.k;
      } else {
        view.x = (right + 24) / 2 - (box.x + box.width / 2) * view.k;
        view.y = top + (bottom - top) / 2 - (box.y + box.height / 2) * view.k;
      }
      apply();
    }

    function zoomBy(f, cx, cy) {
      const { right, bottom } = opts.freeArea();
      cx ??= right / 2; cy ??= bottom / 2;
      const k = Math.min(3, Math.max(0.15, view.k * f));
      view.x = cx - (cx - view.x) * (k / view.k);
      view.y = cy - (cy - view.y) * (k / view.k);
      view.k = k;
      apply();
    }

    // Open every branch along keys (expanding lazy ones) and center the last node.
    function revealPath(keys) {
      let n = tree;
      for (const key of keys) {
        const next = kids(n).find((c) => c.key === key);
        if (!next) break;
        if (next !== tree) open.add(n.key);
        n = next;
      }
      relayout();
      const { top, bottom, right } = opts.freeArea();
      view.k = Math.max(view.k, 0.9);
      view.x = right / 2 - n.x * view.k;
      view.y = (top + bottom) / 2 - n.y * view.k;
      apply();
      return n;
    }

    // Search: open only the branches leading to these nodes (each a key path from the root), fit them,
    // and remember what was open before so clearing the search puts the tree back.
    let beforeSearch = null;
    function revealMany(paths) {
      beforeSearch ??= new Set(open);
      open = new Set();
      for (const keys of paths) {
        let n = tree;
        for (const key of keys) {
          const next = kids(n).find((c) => c.key === key);
          if (!next) break;
          if (n !== tree) open.add(n.key);
          n = next;
        }
      }
      relayout();
      fit(true);
    }
    function endReveal() {
      if (!beforeSearch) return;
      open = beforeSearch;
      beforeSearch = null;
      relayout();
      fit();
    }
    // Center one node without changing what is open.
    function centerOn(key) {
      const n = drawn.get(key);
      if (!n) return null;
      const { top, bottom, right } = opts.freeArea();
      view.k = Math.max(view.k, 0.8);
      view.x = right / 2 - n.x * view.k;
      view.y = (top + bottom) / 2 - n.y * view.k;
      apply();
      return n;
    }

    // pan & zoom
    let drag = null;
    svg.addEventListener('pointerdown', (e) => {
      if (e.target.closest('.mm-node')) return;
      drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, moved: false };
      svg.classList.add('dragging');
      try { svg.setPointerCapture(e.pointerId); } catch { /* already released */ }
    });
    svg.addEventListener('pointermove', (e) => {
      if (!drag) return;
      view.x = drag.vx + e.clientX - drag.x; view.y = drag.vy + e.clientY - drag.y;
      drag.moved ||= Math.abs(e.clientX - drag.x) + Math.abs(e.clientY - drag.y) > 4;
      apply();
    });
    svg.addEventListener('pointerup', () => {
      if (drag && !drag.moved) opts.onSelect?.(null);
      drag = null; svg.classList.remove('dragging');
    });
    svg.addEventListener('wheel', (e) => {
      e.preventDefault();
      const box = svg.getBoundingClientRect();
      zoomBy(Math.exp(-e.deltaY * 0.0015), e.clientX - box.left, e.clientY - box.top);
    }, { passive: false });

    return {
      // keepOpen: rebuild (e.g. after a filter change) without collapsing what the user opened.
      setTree(t, { keepOpen = false } = {}) {
        tree = t;
        annotate([tree], 0, null);
        if (!keepOpen) open = new Set(kids(tree).filter((c) => !c.collapsed).map((c) => c.key));
        relayout();
        if (!keepOpen) fit();
      },
      relayout, fit, zoomBy, revealPath, revealMany, endReveal, centerOn,
      node: (key) => drawn.get(key),
      setSelected(key) { selected = key; mark(); },
      setHighlight(fn) { highlight = fn ?? null; mark(); },
      expandAll(on) {
        open.clear();
        if (on) {
          const walk = (n) => { if (n.children?.length) { open.add(n.key); n.children.forEach(walk); } };
          walk(tree);
        } else kids(tree).forEach((c) => open.add(c.key));
        relayout();
        fit();
      },
      show() { svg.style.display = 'block'; },
      hide() { svg.style.display = 'none'; },
    };
  };
})();
