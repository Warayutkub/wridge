// Turns design_map (+ check_flows) output into { nodes, links } for the force-graph viewer.

export function toGraphData(map, flows) {
  const nodes = [];
  const links = [];
  const flowStarts = new Set(map.pages.flatMap((p) => p.flows.map((f) => f.nodeId)));
  const deadEnds = new Set((flows?.deadEnds ?? []).map((d) => d.id));
  const orphans = new Set((flows?.orphans ?? []).map((o) => o.id));
  const screenIds = new Set();

  for (const page of map.pages) {
    for (const s of page.screens) {
      screenIds.add(s.id);
      nodes.push({
        id: s.id,
        name: s.name,
        type: "screen",
        page: page.name,
        size: s.size,
        flowStart: flowStarts.has(s.id),
        issue: deadEnds.has(s.id) ? "dead-end" : orphans.has(s.id) ? "orphan" : undefined,
      });
    }
  }

  const components = new Set();
  for (const [name, count, screens] of map.components) {
    components.add(name);
    nodes.push({ id: `c:${name}`, name, type: "component", count, screens });
  }
  for (const name of map.unusedComponents) {
    if (!components.has(name)) nodes.push({ id: `c:${name}`, name, type: "component", count: 0, screens: 0, issue: "unused" });
  }

  for (const page of map.pages) {
    for (const s of page.screens) {
      for (const [name, count] of s.components) links.push({ source: s.id, target: `c:${name}`, type: "uses", value: count });
      for (const to of s.toIds ?? []) if (screenIds.has(to)) links.push({ source: s.id, target: to, type: "navigates" });
    }
  }

  for (const [setName, bps] of Object.entries(map.breakpoints)) {
    const ids = Object.values(bps).filter((id) => screenIds.has(id));
    for (let i = 1; i < ids.length; i++) links.push({ source: ids[i - 1], target: ids[i], type: "breakpoint", set: setName });
  }

  return {
    file: map.file,
    pages: map.pages.map((p) => ({ id: p.id, name: p.name })),
    flowsChecked: Boolean(flows),
    nodes,
    links,
  };
}
