// Shared live search for the graph pages (/code, /graph, /data): matches light up while typing,
// a results bar shows counts and steps through them (‹ ›, ↑ ↓), Enter zooms to the current one.
// The page supplies find(q) → hits and apply(state); this file owns the input, the bar and the list.
window.GraphSearch = (() => {
  const MAX = 300; // more hits than this: say the query is too broad instead of dimming everything else
  const DEBOUNCE = 120;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

  /**
   * opts: {
   *   input, bar,                 elements: the search box and an (empty) element for the results bar
   *   find(q) → hit[],            hit: { id, label, kind, sub? } — kind is a key of opts.kinds
   *   kinds: { [kind]: { label, color } }   in display order
   *   apply(state)                state = { q, hits, broad, index } or null when cleared; restyle the graph
   *   focus(hit, state)           zoom/camera to one hit (‹ ›, ↑ ↓, Enter)
   *   empty?(q)                   Enter with no hits (e.g. show the "not found" card)
   * }
   */
  function attach(opts) {
    let state = null, timer = null;
    const { input, bar } = opts;
    bar.classList.add('gs-bar');
    bar.hidden = true;

    function run(q, { zoom = false } = {}) {
      q = q.trim();
      if (!q) return clear({ keepInput: true });
      const hits = opts.find(q.toLowerCase(), q);
      const broad = hits.length > MAX;
      const prev = state?.q === q ? state.index : 0;
      state = { q, hits, broad, index: hits.length ? Math.min(prev, hits.length - 1) : -1 };
      paint();
      opts.apply(broad || !hits.length ? { ...state, hits: [] } : state);
      if (zoom) {
        if (!hits.length) opts.empty?.(q);
        else if (!broad) opts.focus(hits[state.index], state);
      }
    }
    function clear({ keepInput = false } = {}) {
      clearTimeout(timer);
      if (!keepInput) input.value = '';
      const had = state;
      state = null;
      bar.hidden = true;
      bar.innerHTML = '';
      if (had) opts.apply(null);
    }
    function step(d) {
      if (!state?.hits.length || state.broad) return;
      state.index = (state.index + d + state.hits.length) % state.hits.length;
      paint();
      opts.apply(state);
      opts.focus(state.hits[state.index], state);
    }
    function go(i) {
      if (!state?.hits[i]) return;
      state.index = i;
      paint();
      opts.apply(state);
      opts.focus(state.hits[i], state);
    }
    function paint() {
      bar.hidden = false;
      bar.classList.toggle('warn', state.broad);
      bar.classList.toggle('none', !state.hits.length);
      if (state.broad) { bar.innerHTML = `<span>${state.hits.length.toLocaleString()} matches — too broad to highlight. Type a little more.</span>`; return; }
      if (!state.hits.length) { bar.innerHTML = `<span>No match for “${esc(state.q)}”</span>`; return; }
      const counts = {};
      for (const h of state.hits) counts[h.kind] = (counts[h.kind] ?? 0) + 1;
      const kinds = Object.entries(opts.kinds).filter(([k]) => counts[k]).map(([k, v]) => `<span class="gs-kind"><i style="background:${v.color}"></i>${esc(v.label)} ${counts[k]}</span>`).join('');
      bar.innerHTML = `<b>${state.hits.length} found</b>${kinds}<span class="gs-step"><button data-d="-1" aria-label="Previous">‹</button><span>${state.index + 1}/${state.hits.length}</span><button data-d="1" aria-label="Next">›</button></span>`;
      bar.querySelectorAll('[data-d]').forEach((b) => (b.onclick = () => step(Number(b.dataset.d))));
    }

    input.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(() => run(input.value), DEBOUNCE); });
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); clearTimeout(timer); if (state?.q === input.value.trim() && state.hits.length) { if (!state.broad) opts.focus(state.hits[state.index], state); } else run(input.value, { zoom: true }); }
      else if (e.key === 'ArrowDown' && state) { e.preventDefault(); step(1); }
      else if (e.key === 'ArrowUp' && state) { e.preventDefault(); step(-1); }
      else if (e.key === 'Escape') { if (input.value) { e.stopPropagation(); clear(); } else input.blur(); }
    });
    input.addEventListener('search', () => { if (!input.value) clear({ keepInput: true }); }); // the ✕ in type=search

    return {
      state: () => state,
      active: () => Boolean(state && state.hits.length && !state.broad),
      run: (q) => { input.value = q; run(q); },
      rerun: () => { if (state) run(state.q); }, // data reloaded
      clear, step, go,
    };
  }

  // Results grouped by kind, for a side panel / bottom sheet. Rows carry data-hit="<index>".
  function listHTML(state, kinds, { max = 60 } = {}) {
    const groups = Object.entries(kinds).map(([k, v]) => [k, v, state.hits.map((h, i) => [h, i]).filter(([h]) => h.kind === k)]).filter(([, , rows]) => rows.length);
    return groups.map(([, v, rows]) => `<div class="gs-group"><div class="gs-group-head"><i style="background:${v.color}"></i>${esc(v.label)} · ${rows.length}</div>${rows.slice(0, max).map(([h, i]) =>
      `<button class="gs-row${i === state.index ? ' current' : ''}" data-hit="${i}"><span>${esc(h.label)}</span>${h.sub ? `<em>${esc(h.sub)}</em>` : ''}</button>`).join('')}${rows.length > max ? `<div class="gs-more">… ${rows.length - max} more</div>` : ''}</div>`).join('');
  }

  return { attach, listHTML, MAX };
})();
