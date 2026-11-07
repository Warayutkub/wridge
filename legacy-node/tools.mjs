import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { z } from "zod";
import { openGraph, updateGraph } from "./codegraph/build.mjs";
import { syncDesign } from "./codegraph/design.mjs";
import { getKnowledge, listKnowledge, saveKnowledge, setFact } from "./codegraph/knowledge.mjs";
import { decide as memoryDecide, find as memoryFind, handoff as memoryHandoff, note as memoryNote, overview as memoryOverview } from "./codegraph/memory.mjs";
import { writeNote } from "./codegraph/notes.mjs";
import { readNote, restoreNote, trashNote, writeNote as writeBrainNote } from "./codegraph/brain.mjs";
import { compileLottie } from "./lottie.mjs";
import { designPdf, pdfFileName } from "./pdf.mjs";
import { DIAGRAM_TYPES, NODE_KINDS, NODE_STATUS, boardText, layoutDiagram } from "./figjam.mjs";
import { MOTION_EASES, MOTION_PROPS, motionText, motionToSpec, presetsToItems, tracksToItems } from "./motion-figma.mjs";
import { checkText, classify, lint, motionContext, toCss, toFramer, tokens } from "./motion-handoff.mjs";
import { QUERIES, graph as codeGraph, runQuery } from "./codegraph/query.mjs";
import {
  BREAKPOINTS,
  COMPONENT_MAP_FILE,
  annotateComponents,
  formatTokens,
  kebab,
  mergeBreakpoints,
  readComponentMap,
  requireAbsolute,
  writeComponentMap,
  writeFileDeep,
} from "./codegen.mjs";

// ---------- shared schemas ----------

const hex = z.string().regex(/^#([0-9a-f]{6}|[0-9a-f]{8})$/i, "hex color like #FF8866 or #FF886680");
const hexOrNone = z.union([hex, z.literal("none")]);
const nodeId = z.string().describe("Figma node id, e.g. 12:34");
const nodeIds = z.array(z.string()).min(1);
const allowFlowIssues = z.boolean().optional().describe("Only when the user accepts sending this section to review/approval with prototype issues");
const parentId = z.string().optional().describe("Parent frame id; defaults to the current page");
const px = z.number().min(0);
const sizing = z.enum(["FIXED", "HUG", "FILL"]);

const layoutProps = {
  layoutMode: z.enum(["NONE", "HORIZONTAL", "VERTICAL"]).optional(),
  itemSpacing: px.optional().describe("Gap between children"),
  counterAxisSpacing: px.optional().describe("Gap between wrapped rows"),
  wrap: z.boolean().optional().describe("Wrap children (horizontal only)"),
  padding: px.optional().describe("All four sides"),
  paddingX: px.optional(),
  paddingY: px.optional(),
  paddingTop: px.optional(),
  paddingRight: px.optional(),
  paddingBottom: px.optional(),
  paddingLeft: px.optional(),
  primaryAlign: z.enum(["MIN", "CENTER", "MAX", "SPACE_BETWEEN"]).optional().describe("Along the layout direction"),
  counterAlign: z.enum(["MIN", "CENTER", "MAX", "BASELINE"]).optional().describe("Across the layout direction"),
};

const sizingProps = {
  widthSizing: sizing.optional().describe("HUG = fit content, FILL = stretch in auto-layout parent, FIXED"),
  heightSizing: sizing.optional(),
  minWidth: px.optional(),
  maxWidth: px.optional(),
  minHeight: px.optional(),
  maxHeight: px.optional(),
};

const cornerProps = {
  cornerRadius: px.optional(),
  topLeft: px.optional(),
  topRight: px.optional(),
  bottomRight: px.optional(),
  bottomLeft: px.optional(),
};

const lineHeight = z
  .union([z.number().positive(), z.string().regex(/^(auto|\d+(\.\d+)?%)$/i)])
  .describe('px number, "150%", or "auto"');
const letterSpacing = z.union([z.number(), z.string().regex(/^-?\d+(\.\d+)?%$/)]).describe('px number or "-2%"');

const textProps = {
  fontSize: z.number().positive().optional(),
  lineHeight: lineHeight.optional(),
  letterSpacing: letterSpacing.optional(),
  textAlign: z.enum(["LEFT", "CENTER", "RIGHT", "JUSTIFIED"]).optional(),
  autoResize: z.enum(["NONE", "WIDTH_AND_HEIGHT", "HEIGHT", "TRUNCATE"]).optional(),
  width: z.number().positive().optional().describe("Fixed width; text wraps and grows in height"),
  fill: hex.optional(),
};

const BINDABLE_FIELDS = [
  "fill", "stroke",
  "width", "height", "minWidth", "maxWidth", "minHeight", "maxHeight",
  "itemSpacing", "counterAxisSpacing", "paddingLeft", "paddingRight", "paddingTop", "paddingBottom",
  "cornerRadius", "topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius",
  "strokeWeight", "opacity", "visible",
  "fontFamily", "fontSize", "fontStyle", "fontWeight", "letterSpacing", "lineHeight",
];

// ---------- images ----------

const MAX_IMAGE_BYTES = 20 * 1024 * 1024;

function imageFormat(bytes) {
  if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return "png";
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "jpeg";
  if (bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return "gif";
  return null;
}

async function loadImage(source) {
  let bytes;
  if (/^https?:\/\//i.test(source)) {
    const res = await fetch(source);
    if (!res.ok) throw new Error(`Download failed: HTTP ${res.status} for ${source}`);
    bytes = Buffer.from(await res.arrayBuffer());
  } else {
    bytes = await readFile(source);
  }
  if (bytes.length > MAX_IMAGE_BYTES) throw new Error(`Image is ${(bytes.length / 1e6).toFixed(1)} MB; limit is 20 MB`);
  const format = imageFormat(bytes);
  if (!format) throw new Error("Figma only accepts PNG, JPEG or GIF images (WebP/SVG/HEIC are not supported)");
  return { base64: bytes.toString("base64"), format, bytes: bytes.length };
}

// Compact JSON: indentation costs tokens on every result.
const text = (value) => ({ content: [{ type: "text", text: JSON.stringify(value) }] });
const plain = (value) => ({ content: [{ type: "text", text: typeof value === "string" ? value : JSON.stringify(value) }] });

const listWithMore = (items, max, fmt) =>
  items.slice(0, max).map(fmt).join(", ") + (items.length > max ? `, +${items.length - max} more` : "");

// Plain-text overview: far fewer tokens than the equivalent JSON.
function renderDesignMap(map, maxScreens) {
  const screens = map.pages.reduce((n, p) => n + p.screens.length, 0);
  const lines = [
    `File "${map.file}" · ${map.pages.length} page(s) · ${screens} screens · ${map.components.length} components in use · Foundations page: ${map.foundations ? "yes" : "no"}`,
  ];
  const all = map.pages.flatMap((p) => p.screens);
  const count = (st) => all.filter((s) => s.status === st).length;
  lines.push(`Prototype work only (sections "<Flow> · Prototype · Draft / In review / Approved"; old "<Flow> · <Status>" and loose frames count too): ${count("approved")} approved · ${count("review")} in review · ${count("draft")} draft · ${all.filter((s) => !s.status).length} without a review section (not approved)`);
  for (const page of map.pages) {
    lines.push("", `## Page "${page.name}" (${page.id}) — ${page.screens.length} screens`);
    if (page.flows.length) lines.push(`Flows: ${page.flows.map((f) => `${f.name} → ${f.nodeId}`).join(" · ")}`);
    for (const s of page.screens.slice(0, maxScreens)) {
      let line = `- ${s.name} (${s.id}) ${s.size}${s.status ? ` · ${s.status.toUpperCase()}` : ""}`;
      if (s.components.length) line += ` · uses ${listWithMore(s.components, 6, ([n, c]) => (c > 1 ? `${n}×${c}` : n))}`;
      if (s.to.length) line += ` · → ${listWithMore(s.to, 5, (t) => t)}`;
      lines.push(line);
    }
    if (page.screens.length > maxScreens) lines.push(`… ${page.screens.length - maxScreens} more screens (pass pageId or maxScreens)`);
  }
  const sets = Object.entries(map.breakpoints);
  if (sets.length) {
    lines.push("", "## Breakpoint sets");
    for (const [name, bps] of sets) lines.push(`- ${name}: ${Object.entries(bps).map(([bp, id]) => `${bp} ${id}`).join(", ")}`);
  }
  if (map.components.length) {
    lines.push("", "## Components (instances / screens)", map.components.map(([n, c, s]) => `${n} ${c}/${s}`).join(" · "));
  }
  if (map.unusedComponents.length) lines.push("", `Unused local components: ${map.unusedComponents.join(", ")}`);
  if (map.otherWork?.length) lines.push("", "## Other work (not in the design graph)", ...map.otherWork.map((s) => `- ${s.name} [${s.kind}]`));
  return lines.join("\n");
}

// Returns [{ name, description, inputSchema (zod shape), run(args) → MCP result }].
// Plain pass-through tools share the plugin command name.
export function defineTools(send) {
  const tools = [];
  const tool = (name, description, inputSchema, run = async (args) => text(await send(name, args))) =>
    tools.push({ name, description, inputSchema, run });

  // ---- read / discover ----

  tool("get_document_info", "Current file, page list, and the top-level nodes of the current page.", {});

  tool(
    "get_selection",
    "Nodes the user currently has selected, with children down to `depth`.",
    { depth: z.number().int().min(0).max(5).default(1) },
  );

  tool(
    "get_node",
    "Read one node: geometry, fills, images, text, auto layout, sizing, styles, component props, children to `depth`.",
    { nodeId, depth: z.number().int().min(0).max(5).default(2) },
  );

  tool(
    "find_nodes",
    "Search layers by name (case-insensitive substring) and/or type on the current page or inside `withinId`.",
    {
      name: z.string().optional(),
      type: z.string().optional().describe("FRAME, TEXT, INSTANCE, COMPONENT, RECTANGLE, GROUP…"),
      withinId: z.string().optional(),
      limit: z.number().int().min(1).max(500).default(50),
    },
  );

  tool(
    "export_node_image",
    "Render a node to PNG to check how it looks. Use after building each section. Images cost many tokens: keep maxSize small unless you need detail.",
    { nodeId, scale: z.number().min(0.25).max(3).default(1), maxSize: z.number().int().min(128).max(4096).default(1024).describe("Longest side in px; the render is scaled down to fit") },
    async (args) => {
      const { base64 } = await send("export_node_image", args);
      return { content: [{ type: "image", data: base64, mimeType: "image/png" }] };
    },
  );

  tool("get_styles", "Local paint, text and effect styles in this file. Check these before picking raw colors/fonts.", {});

  tool("get_variables", "Local variable collections, modes and variables (colors as hex). Prefer these over raw values.", {});

  tool(
    "get_components",
    "Local components and component sets (with variant options and properties). Reuse these before drawing new UI.",
    { allPages: z.boolean().default(false) },
  );

  // ---- create ----

  tool(
    "create_frame",
    "Create a frame. Auto layout is VERTICAL by default — pass layoutMode NONE only for free positioning.",
    {
      name: z.string().default("Frame"),
      x: z.number().optional(),
      y: z.number().optional(),
      width: z.number().positive().default(375),
      height: z.number().positive().default(812),
      parentId,
      fill: hex.optional().describe("Omit for a transparent frame"),
      clip: z.boolean().optional(),
      ...cornerProps,
      ...layoutProps,
      layoutMode: z.enum(["NONE", "HORIZONTAL", "VERTICAL"]).default("VERTICAL"),
      ...sizingProps,
    },
  );

  tool(
    "create_rectangle",
    "Create a rectangle (dividers, image placeholders, decorative shapes).",
    {
      name: z.string().default("Rectangle"),
      x: z.number().optional(),
      y: z.number().optional(),
      width: z.number().positive().default(100),
      height: z.number().positive().default(100),
      parentId,
      fill: hex.optional(),
      ...cornerProps,
      ...sizingProps,
    },
  );

  tool(
    "create_ellipse",
    "Create an ellipse/circle (avatars, dots, icons backgrounds).",
    {
      name: z.string().default("Ellipse"),
      x: z.number().optional(),
      y: z.number().optional(),
      width: z.number().positive().default(40),
      height: z.number().positive().default(40),
      parentId,
      fill: hex.optional(),
      ...sizingProps,
    },
  );

  tool(
    "create_text",
    "Create a text layer. Use a Thai-capable font (IBM Plex Sans Thai, Noto Sans Thai, Prompt…) for Thai text.",
    {
      text: z.string(),
      name: z.string().optional(),
      x: z.number().optional(),
      y: z.number().optional(),
      parentId,
      fontFamily: z.string().default("Inter"),
      fontStyle: z.string().default("Regular").describe("Regular, Medium, SemiBold, Bold…"),
      ...textProps,
      fontSize: z.number().positive().default(16),
      ...sizingProps,
    },
  );

  tool(
    "create_instance",
    "Place an instance of a component (by componentId from get_components, or a published componentKey).",
    {
      componentId: z.string().optional().describe("COMPONENT or COMPONENT_SET id (a set uses its default variant)"),
      componentKey: z.string().optional(),
      parentId,
      x: z.number().optional(),
      y: z.number().optional(),
      properties: z.record(z.string(), z.union([z.string(), z.boolean()])).optional().describe("Variant/text/boolean props"),
      ...sizingProps,
    },
    async (args) => {
      if (!args.componentId && !args.componentKey) throw new Error("Pass componentId or componentKey");
      return text(await send("create_instance", args));
    },
  );

  tool("create_component", "Turn an existing frame/node into a reusable component.", {
    nodeId,
    name: z.string().optional(),
    description: z.string().optional(),
  });

  tool(
    "combine_as_variants",
    'Combine components into one component set. Name each component "Prop=Value, Prop=Value" first (e.g. "Type=Primary, State=Hover") — those become the variant properties. Lays the set out as a wrapping grid unless layout is false.',
    {
      nodeIds: z.array(z.string()).min(2).describe("COMPONENT ids"),
      name: z.string().min(1).describe('Set name = code component name, e.g. "Button"'),
      parentId,
      layout: z.boolean().default(true),
      width: z.number().positive().default(960),
      gap: z.number().min(0).default(16),
      padding: z.number().min(0).default(24),
    },
  );

  tool(
    "create_section",
    'Wrap top-level frames of one piece of work in a Figma section (free plan; not a page). Name = "<Name> · <Kind> · <Status>": kind Prototype (app screens: in the design graph, prototype checked before review) or Slides / Storyboard / Graphic / anything else (not in the design graph, no prototype check); status Draft, In review or Approved. Change status later with rename_node on the section.',
    {
      name: z.string().min(1).describe('e.g. "Payroll · Draft"'),
      nodeIds: z.array(z.string()).optional().describe("Top-level frames to put inside; the section is sized around them"),
      padding: z.number().min(0).default(80),
      x: z.number().optional(),
      y: z.number().optional(),
      width: z.number().positive().optional(),
      height: z.number().positive().optional(),
      allowFlowIssues,
    },
  );

  tool(
    "add_component_property",
    "Add a TEXT (label), BOOLEAN (show/hide a layer) or INSTANCE_SWAP (icon) property to a component or component set, bound to layers inside it (bindNodeIds; for a set, bind the matching layer in every variant). Mirror the code component's props.",
    {
      componentId: z.string().describe("COMPONENT or COMPONENT_SET id (a variant resolves to its set)"),
      name: z.string().min(1).describe('Property name, e.g. "Label", "Show icon"'),
      type: z.enum(["TEXT", "BOOLEAN", "INSTANCE_SWAP"]),
      defaultValue: z.union([z.string(), z.boolean()]).describe("TEXT: the text; BOOLEAN: true/false; INSTANCE_SWAP: a component id"),
      bindNodeIds: z.array(z.string()).optional(),
    },
  );

  tool("create_paint_style", "Create a local color style (use slash names like Color/Primary/500).", {
    name: z.string().min(1),
    color: hex,
  });

  tool("create_text_style", "Create a local text style (e.g. Text/Heading/H1).", {
    name: z.string().min(1),
    fontFamily: z.string(),
    fontStyle: z.string().default("Regular"),
    fontSize: z.number().positive(),
    lineHeight: lineHeight.optional(),
    letterSpacing: letterSpacing.optional(),
  });

  tool("duplicate_node", "Duplicate a node (optionally into another parent / position).", {
    nodeId,
    parentId,
    x: z.number().optional(),
    y: z.number().optional(),
    name: z.string().optional(),
  });

  tool("group_nodes", "Group nodes (prefer auto-layout frames over groups for UI).", {
    nodeIds,
    name: z.string().optional(),
  });

  tool("ungroup_node", "Ungroup a group.", { nodeId });

  // ---- modify ----

  tool("set_text", "Replace the characters of a text node.", { nodeId, text: z.string() });

  tool("set_text_props", "Change typography of a text node.", {
    nodeId,
    fontFamily: z.string().optional(),
    fontStyle: z.string().optional(),
    ...textProps,
  });

  tool("set_fill", 'Set a single solid fill, or "none" to remove fills.', { nodeId, color: hexOrNone });

  tool("set_stroke", 'Set border color ("none" removes), weight and alignment.', {
    nodeId,
    color: hexOrNone.optional(),
    weight: px.optional(),
    align: z.enum(["INSIDE", "OUTSIDE", "CENTER"]).optional(),
  });

  tool("set_corner_radius", "Set corner radius (all corners or individual).", { nodeId, ...cornerProps });

  tool(
    "set_effects",
    "Replace a node's effects: shadows, layer blur, background blur. Pass an empty shadows list to clear.",
    {
      nodeId,
      shadows: z
        .array(
          z.object({
            color: hex.default("#0000001A"),
            x: z.number().default(0),
            y: z.number().default(4),
            blur: px.default(12),
            spread: z.number().default(0),
            inner: z.boolean().default(false),
          }),
        )
        .default([]),
      layerBlur: px.optional(),
      backgroundBlur: px.optional(),
    },
  );

  tool("set_props", "Set name, visibility, lock, opacity, clip content, absolute positioning inside auto layout, or rotation.", {
    nodeId,
    name: z.string().optional(),
    visible: z.boolean().optional(),
    locked: z.boolean().optional(),
    opacity: z.number().min(0).max(1).optional(),
    clip: z.boolean().optional(),
    absolute: z.boolean().optional().describe("Take the node out of its auto-layout flow"),
    rotation: z.number().min(-360).max(360).optional().describe("Degrees like Figma's rotation field (positive = counter-clockwise), around the node's centre; 0 = upright. Inside auto layout the parent makes room for the rotated box"),
    allowFlowIssues,
  });

  tool("set_auto_layout", "Add or change auto layout on a frame/component (direction, gap, padding, alignment, wrap).", {
    nodeId,
    ...layoutProps,
    ...sizingProps,
  });

  tool("set_sizing", "Set how a node sizes inside auto layout (FIXED/HUG/FILL) and its min/max.", {
    nodeId,
    ...sizingProps,
  });

  tool("move_resize", "Move and/or resize a node. Inside auto layout, prefer set_sizing instead.", {
    nodeId,
    x: z.number().optional(),
    y: z.number().optional(),
    width: z.number().positive().optional(),
    height: z.number().positive().optional(),
  });

  tool("move_to_parent", "Move a node into another parent (optionally at a child index — order matters in auto layout).", {
    nodeId,
    parentId: z.string(),
    index: z.number().int().min(0).optional(),
  });

  tool("rename_node", 'Rename a node. Renaming a section to "· In review" / "· Approved" first checks its prototype (flow start, no dead ends, orphans or broken links) and refuses when it is not wired.', { nodeId, name: z.string().min(1), allowFlowIssues });

  tool("delete_node", "Delete a node. Only nodes made through wridge can be deleted; anything else is the user's work and needs confirm: true, given only after the user agreed.", {
    nodeId,
    confirm: z.boolean().optional().describe("Only after the user explicitly agreed to delete their own node"),
  });

  tool("select_nodes", "Select nodes in Figma and zoom to them — do this at the end so the user sees the result.", {
    nodeIds,
    zoom: z.boolean().default(true),
  });

  // ---- images ----

  tool(
    "set_image_fill",
    "Fill a node with an image from a local file path or an http(s) URL. PNG/JPEG/GIF, max 4096×4096 px.",
    {
      nodeId,
      source: z.string().min(1).describe("Absolute local path (e.g. C:/Users/me/pic.jpg) or https:// URL"),
      scaleMode: z.enum(["FILL", "FIT", "CROP", "TILE"]).default("FILL").describe("FILL = cover, FIT = contain, TILE = repeat"),
    },
    async ({ nodeId, source, scaleMode }) => {
      const { base64, format, bytes } = await loadImage(source);
      const result = await send("set_image_fill", { nodeId, base64, scaleMode });
      return text({ ...result, format, bytes });
    },
  );

  tool("copy_fills", "Copy all fills (images, colors, gradients) from one node onto other nodes.", {
    fromNodeId: nodeId,
    toNodeIds: nodeIds,
  });

  // ---- design → code ----

  const projectDir = z.string().describe(`Absolute path of the code project (holds ${COMPONENT_MAP_FILE})`);

  tool(
    "get_design_context",
    "Read a frame as code-ready data: flex/grid layout, sizing (fill/hug/px), tokens, component instances with their variant states (hover/disabled…), links from prototype interactions, constraints, text and assets. Pass `breakpoints` (e.g. {base: mobileId, md: tabletId, lg: desktopId}) to get one mobile-first tree with per-breakpoint changes.",
    {
      nodeId: nodeId.optional().describe("A single frame; or use breakpoints"),
      // partialRecord: zod 4 makes enum-keyed records exhaustive, which demanded all five sizes.
      breakpoints: z
        .partialRecord(z.enum(BREAKPOINTS), z.string())
        .optional()
        .describe("Frame id per breakpoint, smallest first: base (mobile), sm, md (tablet), lg (desktop), xl"),
      depth: z.number().int().min(0).max(30).default(12),
      expandInstances: z.boolean().default(false).describe("Also describe the insides of component instances"),
      userApproved: z.boolean().optional().describe("Screens must be in a \"<Flow> · Approved\" section; true only when the user said to code this design without that review"),
      projectDir: projectDir.optional(),
    },
    async ({ projectDir: dir, nodeId: id, breakpoints, ...args }) => {
      let context;
      if (breakpoints && Object.keys(breakpoints).length) {
        const contexts = [];
        for (const bp of BREAKPOINTS.filter((b) => breakpoints[b])) {
          contexts.push({ bp, context: await send("get_design_context", { ...args, nodeId: breakpoints[bp] }) });
        }
        context = mergeBreakpoints(contexts);
      } else if (id) {
        context = await send("get_design_context", { ...args, nodeId: id });
      } else {
        throw new Error("Pass nodeId or breakpoints");
      }
      if (dir) context.unmappedComponents = annotateComponents(context.root, await readComponentMap(dir));
      // Figma Motion on this frame: which layers animate and whether they ship as code or Lottie.
      try {
        const r = await send("motion_read", { nodeId: id ?? breakpoints.base ?? Object.values(breakpoints)[0] });
        const rows = classify(r);
        if (rows.length) context.motion = {
          timelineMs: Math.round((r.timelines?.[0]?.duration ?? 0) * 1000),
          layers: rows.map(({ id: layerId, name, to, why, props, span }) => ({ id: layerId, name, to, why, props, span })),
          next: "motion_check → user approves → motion_tokens / motion_to_code (code) and motion_to_lottie (Lottie)",
        };
      } catch { /* no Figma Motion here (editor without the beta API) */ }
      return { content: [{ type: "text", text: JSON.stringify(context) }] };
    },
  );

  tool(
    "export_assets",
    "Export icons (SVG) and images (PNG) from Figma into a folder of the code project. platform flutter writes PNGs as 1x + 2.0x/ + 3.0x/ variants. Returns the written file paths.",
    {
      nodeIds,
      outDir: z.string().describe("Absolute folder, e.g. C:/code/app/public/assets or C:/code/app/assets/images"),
      platform: z.enum(["web", "flutter"]).default("web"),
      format: z.enum(["auto", "svg", "png"]).default("auto").describe("auto = SVG for icons/vectors, PNG otherwise"),
      scale: z.number().min(1).max(4).default(2).describe("Web PNG scale (2 = retina); ignored for flutter"),
    },
    async ({ nodeIds: ids, outDir, platform, format, scale }) => {
      requireAbsolute(outDir, "outDir");
      const flutter = platform === "flutter";
      const exported = await send("export_assets", { items: ids.map((id) => ({ nodeId: id, format, scale: flutter ? 1 : scale })) });
      const used = new Set();
      const files = [];
      const write = async (asset, name, dir) => {
        const file = join(dir, `${name}.${asset.format}`);
        const bytes = Buffer.from(asset.base64, "base64");
        await writeFileDeep(file, bytes);
        files.push({ nodeId: asset.nodeId, file: file.replaceAll("\\", "/"), format: asset.format, bytes: bytes.length });
      };
      for (const asset of exported) {
        const base = kebab(asset.name).replaceAll("-", flutter ? "_" : "-") || `asset_${kebab(asset.nodeId)}`;
        let name = base;
        for (let n = 2; used.has(`${name}.${asset.format}`); n++) name = `${base}_${n}`;
        used.add(`${name}.${asset.format}`);
        await write(asset, name, outDir);
        if (flutter && asset.format === "png") {
          // Flutter picks resolution variants from sibling 2.0x/ and 3.0x/ folders.
          for (const s of [2, 3]) {
            const [variant] = await send("export_assets", { items: [{ nodeId: asset.nodeId, format: "png", scale: s }] });
            await write(variant, name, join(outDir, `${s}.0x`));
          }
        }
      }
      return text(files);
    },
  );

  tool(
    "export_pdf",
    "Download a document/book design as ONE PDF: each frame in the section is a page (vector, text stays selectable), stacked in order. Order = the number at the start of the frame name (1, 2, 10, 100 — compared as numbers, no zero-padding needed); frames without a number follow by position; hidden frames are skipped. A single frame id gives a one-page PDF. No page numbers are added.",
    {
      nodeId: nodeId.describe("Section whose frames are the pages (or one frame)"),
      out: z.string().describe("Absolute .pdf path, or an absolute folder (the file is named after the section)"),
    },
    async ({ nodeId: id, out }) => {
      requireAbsolute(out, "out");
      const { pdf, title, pages, warnings } = await designPdf(send, id);
      const file = /\.pdf$/i.test(out) ? out : join(out, pdfFileName(title));
      await writeFileDeep(file, pdf);
      return text({ file: file.replaceAll("\\", "/"), pages: pages.map((p, i) => `${i + 1}. ${p.name}`), bytes: pdf.length, warnings });
    },
  );

  tool(
    "get_tokens",
    "Build a token file (Tailwind v4 @theme, CSS variables, Flutter Dart constants, or JSON) from the file's Foundations page (layers named as tokens — works on the free plan) and/or its variables and styles. Returns the content and a map from design-context token names to code identifiers; writes it when outFile is given.",
    {
      format: z.enum(["tailwind", "css", "dart", "json"]).default("tailwind").describe("tailwind = Tailwind v4 @theme, css = CSS variables, dart = Flutter constants"),
      source: z.enum(["auto", "foundations", "file"]).default("auto").describe("auto = Foundations page + variables/styles"),
      outFile: z.string().optional().describe("Absolute path to write, e.g. C:/code/app/app/tokens.css"),
    },
    async ({ format, source, outFile }) => {
      const data = await send("get_tokens", { source });
      const { content, map } = formatTokens(data, format);
      if (outFile) await writeFileDeep(requireAbsolute(outFile, "outFile"), content);
      return text({ sources: data.sources, written: outFile ?? null, map, content });
    },
  );

  tool(
    "scan_design_values",
    "List the colors, typography, radii, gaps, paddings and shadows actually used inside a frame, most frequent first — use it to name tokens / build a Foundations page for a file that has none.",
    { nodeId, limit: z.number().int().min(1).max(50).default(12) },
  );

  tool("get_component_map", `Read ${COMPONENT_MAP_FILE}: which Figma components map to which code components (or CSS classes).`, { projectDir }, async ({ projectDir: dir }) =>
    text(await readComponentMap(dir)),
  );

  tool(
    "map_component",
    `Record (or remove) a Figma component → code mapping in ${COMPONENT_MAP_FILE} so future code generation reuses it. Either a code component (import + component) or, for projects whose UI is shared CSS classes and no component exists yet, an element + className (e.g. "Button/Type=Primary" → <button className="btn btn-p">). New projects always use components.`,
    {
      projectDir,
      figmaName: z.string().describe('Component (set) name as in Figma, e.g. "Button" or "Button/Type=Primary"'),
      import: z.string().optional().describe('Module path, e.g. "@/components/ui/button"'),
      component: z.string().optional().describe('Export name, e.g. "Button"'),
      element: z.string().optional().describe('Class mapping: HTML element, e.g. "button"'),
      className: z.string().optional().describe('Class mapping: base classes, e.g. "btn"'),
      variants: z.record(z.string(), z.string()).optional().describe('Class mapping: extra classes per variant value, e.g. {"Type=Primary": "btn-p", "Type=Danger": "btn-d", "Size=Small": "btn-sm"}; map the whole set once under its set name'),
      props: z.record(z.string(), z.unknown()).optional().describe("Notes on how Figma props map to code props / extra classes"),
      remove: z.boolean().default(false),
    },
    async ({ projectDir: dir, figmaName, remove, ...entry }) => {
      const map = await readComponentMap(dir);
      if (remove) delete map[figmaName];
      else {
        const isComponent = entry.import && entry.component;
        if (!isComponent && !entry.className) throw new Error("Pass import + component, or className (+ element), unless remove is true");
        if (isComponent && entry.className) throw new Error("Pass either import + component or className, not both");
        map[figmaName] = Object.fromEntries(Object.entries(entry).filter(([, v]) => v !== undefined));
      }
      const file = await writeComponentMap(dir, map);
      return text({ file: file.replaceAll("\\", "/"), entries: Object.keys(map).length });
    },
  );

  // ---- code graph (runs in the server, no Figma needed) ----

  tool(
    "code_graph",
    "Ask the code graph of a project instead of opening files, and before writing code: what exists to reuse (find also searches tables/columns) and what a change affects (impact). overview | find <text> | file <path> | used_by <target> | uses <target> | impact <target> | route </url or ScreenWidget> | design (Figma components ↔ code) | tokens [token] (design tokens: where used, unused, raw colors, differs from Figma) | notes (AI-written notes: stale ones, and where a note is missing) | data (tables/views/functions from migrations, SQL files or schema strings in code; RLS, who reads/writes them, drift) | table <name> (columns, indexes, relations, policies, every query using it) | flows [name] (agreed behaviour from docs/flows/, ⚠ steps whose code is gone, and drafts for entry points no flow covers — read the flow before exploring code) | trace <route or symbol> (live: calls, tables read/written, packages from there). Target = symbol name (Button), path (components/ui/button.tsx), path#Name, route, or for impact a table / table.column (orders.status). Updates itself for changed files first.",
    {
      projectDir: z.string().describe("Absolute path of the code project"),
      query: z.enum(QUERIES),
      target: z.string().optional(),
      depth: z.number().int().min(1).max(8).optional().describe("impact: reverse-dependency levels (default 3); route: render-tree depth (default 3)"),
      limit: z.number().int().min(5).max(500).default(40),
    },
    async ({ projectDir: dir, query, target, depth, limit }) => {
      const { db, info } = await openGraph(dir);
      try {
        const head = info.gitignore?.added ? `(added ${info.gitignore.pattern} to ${info.gitignore.gitignore})\n` : "";
        return { content: [{ type: "text", text: head + runQuery(db, query, target, { depth, limit, root: dir }) }] };
      } finally {
        db.close();
      }
    },
  );

  tool(
    "code_graph_update",
    "Build or refresh <projectDir>/.wara/code-graph.db (adds .wara/ to .gitignore first). code_graph already refreshes itself; use this for the first build, after big changes, or force=true to re-parse everything.",
    { projectDir: z.string().describe("Absolute path of the code project"), force: z.boolean().default(false) },
    async ({ projectDir: dir, force }) => text(await updateGraph(dir, { force })),
  );

  tool(
    "code_graph_note",
    "Save what a folder, file or symbol is FOR (purpose, rules, caveats) — 1-3 sentences, not a summary of the code. Target: dir:<folder> | <file path> | path#Name | unique symbol name. Empty text deletes. Notes show up in code_graph answers and are flagged stale when that code changes.",
    {
      projectDir: z.string().describe("Absolute path of the code project"),
      target: z.string(),
      text: z.string().max(1200),
    },
    async ({ projectDir: dir, target, text: note }) => {
      const { db } = await openGraph(dir);
      try {
        return text(writeNote(db, codeGraph(db), target, note));
      } finally {
        db.close();
      }
    },
  );

  // ---- Figma Motion (beta) ----

  tool(
    "motion_read",
    "Figma Motion: read the timeline animation of a frame/layer (selection when nodeId is omitted) and everything inside it — per layer, each animated field (OPACITY, TRANSLATION_X/Y, SCALE_X/Y, ROTATION, PATH_TRIM_END, WIDTH, fills[0] colour…) with its keyframes (seconds), easing (EASE_OUT, BOUNCY, cubic(…), HOLD…) and applied animation styles. styles: true also lists the Figma animation styles (presets) the file offers.",
    { nodeId: z.string().optional(), styles: z.boolean().default(false) },
    async (args) => ({ content: [{ type: "text", text: motionText(await send("motion_read", args)) }] }),
  );

  tool(
    "motion_animate",
    "Figma Motion: animate layers inside a frame on its timeline — keyframe tracks in the same short language as create_lottie (layer by name / glob / \"all\" / \"root\", prop, from/to, start + end or duration in ms, ease, stagger ms between matched layers, or keys [[ms, value, ease?]]) and/or Figma's own presets (Opacity, Position, Scale, Rotation, Size, Path, TextStagger; motion_read styles:true lists their props). The designer can then tweak it on the timeline. The result is read back. Load the motion skill first (code vs Lottie, motion tokens, taste).",
    {
      nodeId: z.string().describe("The frame whose timeline you animate"),
      duration: z.number().positive().optional().describe("Timeline length in ms"),
      replace: z.boolean().default(false).describe("Remove existing manual tracks and styles on the targeted layers first"),
      tracks: z.array(z.object({
        layer: z.string().default("all"), prop: z.enum(MOTION_PROPS), from: z.any().optional(), to: z.any().optional(),
        start: z.number().min(0).optional(), end: z.number().min(0).optional(), duration: z.number().min(0).optional(),
        ease: z.union([z.string(), z.array(z.number()).length(4)]).optional().describe(MOTION_EASES.join(" | ")),
        stagger: z.number().min(0).optional(), keys: z.array(z.array(z.any()).min(2).max(3)).optional(),
      })).default([]),
      presets: z.array(z.object({ layer: z.string().default("all"), style: z.string(), props: z.record(z.string(), z.any()).optional(), delay: z.number().min(0).optional(), duration: z.number().positive().optional(), stagger: z.number().min(0).optional() })).default([]),
    },
    async ({ nodeId: id, duration, replace, tracks, presets }) => {
      const items = tracksToItems(tracks);
      if (presets.length) {
        const { animationStyles } = await send("motion_read", { nodeId: id, styles: true });
        items.push(...presetsToItems(presets, animationStyles ?? []));
      }
      const out = await send("motion_animate", { rootId: id, items, replace, ...(duration ? { duration: duration / 1000 } : {}) });
      const lines = [`Animated ${out.applied.length} track(s)/style(s) under ${id}.`];
      if (out.missing.length) lines.push(`⚠ no layer matched: ${out.missing.join(", ")} (layers: ${out.layers.join(", ")})`);
      if (out.idChanges?.length) lines.push(`⚠ Figma gave the frame's layers new ids ${out.idChanges.join("; ")} — re-read ids (get_node) before using old ones`);
      lines.push("", motionText(await send("motion_read", { nodeId: id })));
      return { content: [{ type: "text", text: lines.join("\n") }] };
    },
  );


  // Read one or more frames' Motion and classify every animated layer (code + tokens vs Lottie).
  // The section a frame sits in decides the context: a "… · Storyboard · …" / landing section is landing motion.
  async function sectionOf(id) {
    const doc = await send("get_document_info");
    for (const top of doc.topLevel ?? []) {
      if (top.type !== "SECTION") continue;
      const sec = await send("get_node", { nodeId: top.id, depth: 1 });
      if ((sec.children ?? []).some((c) => c.id === id)) return top.name;
    }
    return null;
  }
  async function handoff(ids, context = "auto") {
    const warnings = [], rows = [], reads = [];
    if (context === "auto") context = motionContext(await sectionOf(ids[0]).catch(() => null));
    for (const id of ids) {
      const r = await send("motion_read", { nodeId: id });
      reads.push(r);
      rows.push(...classify(r, warnings, { context }));
    }
    return { reads, rows, warnings, context };
  }
  const contextParam = z.enum(["auto", "ui", "landing"]).default("auto").describe("ui = screens in an app; landing = landing / product / brand pages (long expressive moments, art is part of a code-driven scroll scene). auto reads the frame's section name (Storyboard / landing → landing)");

  tool(
    "motion_check",
    "Figma Motion handoff, step 1: for a frame (screen or illustration) list every animated layer and decide code + motion tokens (UI: text, components, auto-layout, images, simple transforms) or Lottie (vector art with path drawing / colour / multi-step motion, and Lottie/… components), plus a lint of the UI motion (too slow, width/height, too many durations/easings, mixed frames). Show the table to the user and get approval before motion_tokens / motion_to_code / motion_to_lottie.",
    { nodeId: z.string().describe("Frame with a Figma Motion timeline"), context: contextParam },
    async ({ nodeId: id, context: ctx }) => {
      const { reads, rows, warnings, context } = await handoff([id], ctx);
      const text = checkText(reads[0], rows, lint(rows, reads[0], { context }), context);
      return { content: [{ type: "text", text: [text, ...warnings.map((w) => `⚠ ${w}`)].join("\n") }] };
    },
  );

  tool(
    "motion_tokens",
    "Figma Motion handoff: the motion tokens (durations + easings) used by the code-bound animations of one or more frames, as CSS variables and JSON. Run it over all the screens of a flow so they share one set; if the project already has motion tokens, map onto those instead of adding new ones (motion skill).",
    { nodeIds: z.array(z.string()).min(1).describe("Frames with Figma Motion timelines"), context: contextParam },
    async ({ nodeIds, context }) => {
      const { rows, warnings } = await handoff(nodeIds, context);
      const tk = tokens(rows);
      if (!Object.keys(tk.duration).length) return { content: [{ type: "text", text: "No code-bound animation in these frames (motion_check shows what goes to Lottie)." }] };
      return { content: [{ type: "text", text: ["## CSS", tk.css, "", "## JSON (tokens file / Figma variables)", tk.json, ...warnings.map((w) => `⚠ ${w}`)].join("\n") }] };
    },
  );

  tool(
    "motion_to_code",
    "Figma Motion handoff: code for the code-bound animations of a frame (after motion_check is approved) — CSS @keyframes using the motion token variables, or Framer Motion elements with exact per-prop key times and easings. Reduced motion is handled. Adapt the snippet to the project's components; Lottie layers are left to motion_to_lottie.",
    {
      nodeId: z.string(),
      target: z.enum(["css", "framer-motion"]).default("css"),
      tokenFrames: z.array(z.string()).optional().describe("Frames whose motion shares the token set (default: this one)"),
      context: contextParam,
    },
    async ({ nodeId: id, target, tokenFrames, context: ctx }) => {
      const { rows, warnings, context } = await handoff([id], ctx);
      const tk = tokens(tokenFrames?.length ? (await handoff(tokenFrames, context)).rows : rows);
      if (!rows.some((x) => x.to === "code")) return { content: [{ type: "text", text: "Nothing here goes to code — every animated layer is Lottie (motion_to_lottie)." }] };
      const code = target === "css" ? toCss(rows, tk, warnings) : toFramer(rows, tk);
      const skipped = rows.filter((x) => x.to === "lottie").map((x) => x.name);
      return { content: [{ type: "text", text: ["```" + (target === "css" ? "css" : "jsx"), code, "```", ...(skipped.length ? [`Lottie (not in this code): ${skipped.join(", ")}`] : []), ...warnings.map((w) => `⚠ ${w}`)].join("\n") }] };
    },
  );

  // ---- FigJam (system design boards) ----

  tool(
    "figjam_diagram",
    "FigJam: draw a whole system-design diagram in one call (type flow | state | er | sequence) — you give boxes and arrows (or tables, or participants + messages), the server lays them out (lanes = who does each step, columns = order) inside a new section placed clear of existing work. Kinds: start, end, step, wait (⏳ waiting on someone/something), decision (diamond; label its outgoing edges), state (for state machines: edges = transitions), table (data). status: existing (grey, already in the product), new (blue, default), changed (amber), removed (red). notes become stickies beside it: question (yellow), risk (red), agreed (green), note (grey). Loops (retry, go back) are drawn dashed. Load the system-design skill first.",
    {
      title: z.string().describe('Section name, "<feature> · System design · Draft"'),
      type: z.enum(DIAGRAM_TYPES).default("flow").describe("flow: activity diagram with lanes · state: state machine (nodes default to kind state, edges = transitions with the event as label) · er: tables → arrows from FK to referenced table · sequence: participants + time-ordered messages"),
      tables: z.array(z.object({ id: z.string().optional(), name: z.string(), status: z.enum(NODE_STATUS).optional(), columns: z.array(z.object({ name: z.string(), type: z.string().optional(), pk: z.boolean().optional(), fk: z.string().optional().describe("table.column it references"), note: z.string().optional() })) })).optional().describe("er only"),
      participants: z.array(z.string()).optional().describe("sequence only, left to right"),
      messages: z.array(z.object({ from: z.string(), to: z.string(), text: z.string().max(80), kind: z.enum(["call", "reply", "async"]).optional() })).optional().describe("sequence only, in time order; reply = dashed"),
      lanes: z.array(z.string()).optional().describe("Actors top to bottom, e.g. ร้านค้า, หน้าเว็บ, server, Slip2Go, LINE OA"),
      nodes: z.array(z.object({ id: z.string(), text: z.string().max(140).default("").describe("Empty for start / end"), kind: z.enum(NODE_KINDS.filter((k) => !["question", "risk", "agreed", "note"].includes(k))).optional(), lane: z.string().optional(), status: z.enum(NODE_STATUS).optional() })).default([]).describe("flow / state"),
      edges: z.array(z.object({ from: z.string(), to: z.string(), label: z.string().max(60).optional(), dashed: z.boolean().optional() })).default([]),
      notes: z.array(z.object({ text: z.string().max(300), kind: z.enum(["question", "risk", "agreed", "note"]).default("note") })).default([]),
      x: z.number().optional(),
      y: z.number().optional(),
    },
    async (spec) => {
      const draw = await layoutDiagram(spec);
      // FigJam draws slowly (a few seconds per dozen connectors): give it time instead of the usual 30 s.
      const { ids, section } = await send("figjam_draw", draw, 180_000);
      return { content: [{ type: "text", text: `Drew "${section?.name}" (${section?.id}) · ${draw.nodes.length} items, ${draw.edges.length} arrows${draw.lanes.length ? `, lanes: ${draw.lanes.map((l) => l.name).join(" | ")}` : ""}.
Node ids → FigJam ids: ${Object.entries(ids).filter(([k]) => k !== "$section").map(([k, v]) => `${k}=${v}`).join(", ")}
Read edits back later with board_map.` }] };
    },
  );

  tool(
    "board_map",
    "FigJam: read a board (or one section) back as compact text — every step/sticky with its lane and text, every arrow as from → to [label], using the ids given in figjam_diagram. Use it after the user edited the board, before writing the flow file.",
    { sectionId: z.string().optional() },
    async (args) => ({ content: [{ type: "text", text: boardText(await send("board_map", args)) }] }),
  );

  tool("create_sticky", "FigJam: one sticky note (color hex, wide for a wide sticky).", {
    text: z.string(), x: z.number().optional(), y: z.number().optional(), color: hex.optional(), wide: z.boolean().optional(), parentId: z.string().optional(), name: z.string().optional(),
  });

  tool("create_shape", "FigJam: one shape with text (square, ellipse, rounded rectangle, diamond…). For whole diagrams use figjam_diagram.", {
    text: z.string(), shape: z.enum(["SQUARE", "ELLIPSE", "ROUNDED_RECTANGLE", "DIAMOND", "TRIANGLE_UP", "TRIANGLE_DOWN", "PARALLELOGRAM_RIGHT", "PARALLELOGRAM_LEFT", "ENG_DATABASE", "ENG_QUEUE", "ENG_FILE", "ENG_FOLDER"]).default("ROUNDED_RECTANGLE"),
    x: z.number().optional(), y: z.number().optional(), width: z.number().positive().optional(), height: z.number().positive().optional(), fill: hex.optional(), stroke: hex.optional(), textColor: hex.optional(), parentId: z.string().optional(), name: z.string().optional(),
  });

  tool("create_connector", "FigJam: an arrow between two existing nodes (it follows them when they move).", {
    from: z.string(), to: z.string(), label: z.string().optional(), dashed: z.boolean().optional(), color: hex.optional(), lineType: z.enum(["ELBOWED", "STRAIGHT", "CURVED"]).optional(), parentId: z.string().optional(),
  });

  // Compile and write a Lottie into the project; the recipe rides along in meta (players ignore it):
  // /motion shows it, an edit starts from it. A GIF already sent to Figma stays linked; /motion marks it
  // stale until it is sent again.
  async function writeLottie({ dir, out, id, svg, svgFile, spec, source = "spec", extraWarnings = [] }) {
    const root = resolve(dir);
    const target = resolve(root, out);
    if (relative(root, target).startsWith("..") || isAbsolute(relative(root, target))) throw new Error(`out must be inside ${root}`);
    let art = svg;
    if (!art && svgFile) art = await readFile(isAbsolute(svgFile) ? svgFile : join(root, svgFile), "utf8");
    if (!art && id) {
      const [asset] = await send("export_assets", { items: [{ nodeId: id, format: "svg", scale: 1 }] });
      art = Buffer.from(asset.base64, "base64").toString("utf8");
    }
    if (!art) throw new Error("Give the art: nodeId, svg or svgFile");
    const { lottie, summary, warnings } = compileLottie({ ...spec, name: spec.name ?? out.split(/[\\/]/).pop().replace(/\.json$/i, ""), svg: art });
    let previous = null;
    try { previous = JSON.parse(await readFile(target, "utf8")).meta?.wara?.figma ?? null; } catch { /* new file */ }
    lottie.meta = { g: "wara-bridge", wara: { source, spec: { ...spec, nodeId: id, svgFile }, ...(previous ? { figma: previous } : {}) } };
    const json = JSON.stringify(lottie);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, json);
    return { content: [{ type: "text", text: [
      `Wrote ${relative(root, target).replaceAll("\\", "/")} (${(json.length / 1024).toFixed(1)} KB)${source === "figma-motion" ? " from the Figma Motion timeline" : ""}`,
      summary,
      ...[...extraWarnings, ...warnings].map((w) => `⚠ ${w}`),
      "Play it with lottie-react / lottie-web (web) or the lottie package (Flutter); loop is a player option.",
    ].join("\n") }] };
  }

  tool(
    "create_lottie",
    "Make a Lottie animation (splash/logo, success check, loading, empty state, animated icon) from vector art + a short motion spec; the server writes the keyframes. Art: nodeId (a Figma frame — its top-level layers become the animatable layers, named as in Figma), svg markup, or svgFile. Track: layer (name, glob like 'dot*', or 'all') + prop: opacity 0–1 | scale (1 = 100%, or [x,y]) | move [dx,dy] px | rotate deg | draw 0–1 (strokes draw on) | fill / stroke colour; from/to (default = the resting value), start + end or duration in ms, ease (standard, out, in-out, emphasized, back, spring, bounce, hold, linear, cubic(x1,y1,x2,y2)) — an ease shapes the segment leaving its key, stagger ms between matched layers — or keys [[ms, value, ease?], …]. UI motion (page transitions, sheets, lists, buttons) belongs in code with motion tokens, not Lottie: load the motion skill first.",
    {
      projectDir: z.string().describe("Absolute path of the code project"),
      out: z.string().regex(/\.json$/i).describe("Path inside the project, e.g. assets/lottie/success.json (Flutter) or public/lottie/success.json (web)"),
      nodeId: z.string().optional().describe("Figma node to take the art from"),
      svg: z.string().optional(),
      svgFile: z.string().optional().describe("Path of an .svg file (absolute, or inside the project)"),
      name: z.string().optional(),
      width: z.number().positive().optional().describe("Output size; default = the art's size"),
      height: z.number().positive().optional(),
      duration: z.number().positive().max(60_000).describe("Total length in ms"),
      fps: z.number().int().min(12).max(120).default(60),
      poster: z.number().min(0).optional().describe("ms of the frame shown when not playing (cards, reduced motion, the Figma GIF's first frame). Default: the last frame — set it for loops that fade out at the end"),
      tracks: z.array(z.object({
        layer: z.string().default("all"),
        prop: z.enum(["opacity", "scale", "move", "rotate", "draw", "fill", "stroke"]),
        from: z.any().optional(),
        to: z.any().optional(),
        start: z.number().min(0).optional(),
        end: z.number().min(0).optional(),
        duration: z.number().min(0).optional(),
        ease: z.union([z.string(), z.array(z.number()).length(4)]).optional(),
        stagger: z.number().min(0).optional(),
        keys: z.array(z.array(z.any()).min(2).max(3)).optional(),
      })).default([]),
    },
    async ({ projectDir: dir, out, nodeId: id, svg, svgFile, ...spec }) => writeLottie({ dir, out, id, svg, svgFile, spec }),
  );

  tool(
    "motion_to_lottie",
    "Figma Motion → Lottie: turn the animation a frame has on its Figma Motion timeline (made with motion_animate or by the designer) into a Lottie file in the code project. Art = the frame's SVG; tracks = what motion_read sees on its top-level layers (rotation, scale, move, opacity, path draw, fill/stroke colour; Figma springs become curves). Use it for illustrations / icons that animate as one piece — UI motion goes to code + motion tokens (motion skill). The recipe is kept in meta, so the file can be rebuilt after the design changes.",
    {
      projectDir: z.string().describe("Absolute path of the code project"),
      out: z.string().regex(/\.json$/i).describe("Path inside the project, e.g. public/lottie/success.json"),
      nodeId: z.string().describe("The frame with the Motion timeline"),
      name: z.string().optional(),
      width: z.number().positive().optional(), height: z.number().positive().optional(),
      fps: z.number().int().min(12).max(120).default(60),
      poster: z.number().min(0).optional().describe("ms of the still frame (default: the last frame)"),
    },
    async ({ projectDir: dir, out, nodeId: id, ...opts }) => {
      const read = await send("motion_read", { nodeId: id });
      const { duration, tracks, warnings, split } = motionToSpec(read);
      if (!tracks.length) throw new Error(`Nothing to export: ${warnings.join("; ") || "no Motion animation on this frame's layers"} — animate it first (motion_animate)`);
      return writeLottie({ dir, out, id, spec: { ...opts, duration, tracks, ...(split ? { split } : {}) }, source: "figma-motion", extraWarnings: warnings });
    },
  );

  tool(
    "code_design_sync",
    "Copy the open Figma file's components and Foundations/variable tokens into the project's code graph (needs the Figma plugin). Then code_graph design / tokens can link them to code even when Figma is closed.",
    { projectDir: z.string().describe("Absolute path of the code project") },
    async ({ projectDir: dir }) => {
      const { db } = await openGraph(dir);
      try {
        return text(await syncDesign(db, send));
      } finally {
        db.close();
      }
    },
  );

  tool(
    "project_knowledge",
    "Researched guidance for this project's tools, stored in <project>/.wara/knowledge/<topic>/<tool>@<major>.md (gitignored). list: the detected stack (package.json / go.mod versions), project facts, saved notes, stale ones, and with topic the tools still missing a note. get: one note. save: after researching (official docs first; 1–2 searches, ≤ 3 fetched pages — release/advisory pages and the needed guide section, not whole manuals), a short note with its source URLs. fact: store a user's answer about the project (key + value, e.g. deploy, redis, ci; empty value deletes) so it is asked only once. Reuse notes/facts instead of searching or asking again; re-research only when missing or stale.",
    {
      projectDir: z.string().describe("Absolute path of the code project"),
      action: z.enum(["list", "get", "save", "fact"]),
      key: z.string().optional().describe('fact: e.g. "deploy", "redis", "queue", "ci"'),
      value: z.string().optional().describe('fact: the user\'s answer, e.g. "VPS + Docker Compose, 1 server"'),
      topic: z.string().optional().describe('e.g. "security", "cache"'),
      tool: z.string().optional().describe('Package / module as in the stack list, e.g. "next", "github.com/gofiber/fiber"'),
      version: z.string().optional().describe("save: defaults to the detected version"),
      content: z.string().max(8000).optional().describe("save: the rules for this tool at this version — concrete APIs and settings, not generic advice"),
      sources: z.array(z.string().url()).optional().describe("save: URLs the note is based on (at least one)"),
    },
    async ({ projectDir: dir, action, key, value, topic, tool: name, version, content, sources }) => {
      if (action === "list") return text(listKnowledge(dir, topic));
      if (action === "fact") {
        if (!key || value === undefined) throw new Error("fact needs key and value");
        return text(setFact(dir, key, value));
      }
      if (!topic || !name) throw new Error("topic and tool are required for get/save");
      if (action === "get") return text(getKnowledge(dir, topic, name));
      if (!content || !sources?.length) throw new Error("save needs content and at least one source URL");
      return text(saveKnowledge(dir, { topic, tool: name, version, content, sources }));
    },
  );

  tool(
    "tasks",
    "The project's task board, shared with the person (wridge → Tasks; file docs/tasks/board.json). list (filters: status, label, from, query) · get id · add tasks[] · update id + set · move id + status (+ index) · tick id + subtask · note id + text · accept / skip id (+ text = the reason) · remove id. Found something to fix while checking? add it with change {where, old, new} and from \"<check> · <date>\": it waits in Review until the person accepts; after accept make the change, then move it to done. Never raise a skipped one again (list status: skipped shows the reasons). Before first use: get_skill(\"reports-tasks\").",
    {
      projectDir: z.string().describe("Absolute path of the project"),
      action: z.enum(["list", "get", "add", "update", "move", "tick", "note", "accept", "skip", "remove"]),
      id: z.string().optional().describe('e.g. "T-3"'),
      tasks: z
        .array(
          z.object({
            title: z.string(),
            description: z.string().optional().describe("Markdown"),
            status: z.enum(["todo", "doing", "review", "done"]).optional().describe("default todo; review when change is given"),
            labels: z.array(z.string()).optional(),
            subtasks: z.array(z.string()).optional(),
            links: z.array(z.string()).optional().describe("file:line, docs/flows/x.md, figma:12:34, a PR or https URL"),
            due: z.string().optional(),
            who: z.string().optional().describe("who does it: AI or a name"),
            from: z.string().optional().describe('the batch, e.g. "AI check · 7 Nov"'),
            change: z.object({ where: z.string().optional(), old: z.string(), new: z.string() }).optional().describe("a suggested edit the person accepts or skips"),
          }),
        )
        .optional()
        .describe("add: one or more tasks"),
      set: z.record(z.string(), z.any()).optional().describe("update: the fields to replace (names as in tasks[])"),
      status: z.enum(["todo", "doing", "review", "done", "skipped", "all"]).optional().describe("move: the new column · list: filter (skipped are hidden unless asked)"),
      index: z.number().int().min(0).optional().describe("move: position in the column"),
      subtask: z.number().int().min(1).optional().describe("tick: its number, as get shows"),
      done: z.boolean().optional().describe("tick: false unticks"),
      text: z.string().max(2000).optional().describe("note: the note · skip: the reason"),
      label: z.string().optional().describe("list filter"),
      from: z.string().optional().describe("list filter: the batch name contains this"),
      query: z.string().optional().describe("list filter: words in title or description"),
    },
    async () => {
      throw new Error("Tasks need the Rust engine — switch to Rust in wridge → Settings → Server");
    },
  );

  tool(
    "report",
    "Reports the person reads in wridge → Reports and exports as image or PDF — no Figma needed (docs/reports/<name>.json). write: report {title, subtitle?, eyebrow?, status?, theme?, rows: [block | [block, block] (≤3 side by side)]}; blocks: header, stats, compare, chart (kind line|bars|hbars|stacked|area|donut|pie|scatter|gauge|heatmap|funnel), board, table (preset basic|status|numbers|progress|compare|grouped), timeline, callout, checklist, flow, text, image, tasks (live from the Tasks board: {type: \"tasks\", filter: {from|label|status}}), divider. Send content only — colors, fonts, sizes and layout have defaults; set theme / block style / item colors only when asked. theme: save tokens under name, then \"theme\": \"<name>\". list · get name · remove name · export name + format png|pdf. Load get_skill(\"reports-tasks\") for each block's fields before the first report.",
    {
      projectDir: z.string().describe("Absolute path of the project"),
      action: z.enum(["list", "get", "write", "remove", "theme", "export"]),
      name: z.string().optional().describe("file name in docs/reports/ (write: defaults to the title)"),
      report: z.record(z.string(), z.any()).optional().describe("write: {title, rows: [...]} — see the skill for each block"),
      mode: z.enum(["create", "replace"]).optional().describe("write: default create (fails if the name exists)"),
      theme: z.record(z.string(), z.any()).optional().describe('theme: tokens to save under name, e.g. {"accent": "#7A3BC4"}'),
      format: z.enum(["png", "pdf"]).optional().describe("export: default png"),
    },
    async () => {
      throw new Error("Reports need the Rust engine — switch to Rust in wridge → Settings → Server");
    },
  );

  tool(
    "project_memory",
    `What was agreed and where work stands, kept outside the chat so compaction or a new session loses nothing — for any project (app, game, book, study…), code or not. overview: open tasks, decision titles, brain tags — start here. find: matching decisions, working notes and brain notes (one line each). decide: a project-level decision the user agreed to → docs/decisions.md. note: a short agreement made while working on a task (.wara). handoff: create/update a task's state; close: true when finished. Brain = the project's knowledge as Markdown files in brain/ that the user reads too (Obsidian-style [[links]]: [[Note]], [[code:path#Name]], [[figma:12:34]], [[roblox:Path]], [[project:Note]]; a front-matter key with links is a named relation, e.g. boss_of: [[CH1]]). read: one whole note + its links and backlinks. write: name + content (mode create | append | replace; replace keeps the old text in the trash). trash: move a note to brain/.trash (restore with path). Only the project of this session; another project only when the user asks. Search before reading; write only what was decided or is hard to find again, not chat trivia. Never open docs/decisions.md or brain/ files whole — use find/read.`,
    {
      projectDir: z.string().describe("Absolute path of the project"),
      action: z.enum(["overview", "find", "decide", "note", "handoff", "read", "write", "trash", "restore"]),
      name: z.string().optional().describe('read/write/trash: note name (file name, title or alias); a subfolder is allowed, e.g. "characters/Nathee"'),
      content: z.string().max(20000).optional().describe("write: Markdown; starts with # Title unless front matter is given"),
      mode: z.enum(["create", "append", "replace"]).optional().describe("write: default create (fails if the note exists)"),
      path: z.string().optional().describe("restore: the trash path that trash returned"),
      query: z.string().optional().describe("find: topic words"),
      includeOld: z.boolean().optional().describe("find: also replaced decisions and closed tasks' notes"),
      task: z.string().optional().describe('note/handoff: short task name, e.g. "edupro Foundations in Figma"'),
      title: z.string().optional().describe("decide: short title"),
      decision: z.string().optional().describe("decide: what was decided, 1–3 sentences"),
      why: z.string().optional().describe("decide: the reason"),
      supersedes: z.string().optional().describe('decide: id it replaces, e.g. "D-3"'),
      text: z.string().max(600).optional().describe("note: the agreement, 1–2 sentences"),
      tags: z.array(z.string()).optional(),
      stage: z.string().optional().describe('handoff: e.g. "5/8 Prototype"'),
      done: z.string().optional(),
      next: z.string().optional(),
      questions: z.string().optional().describe("handoff: what waits on the user"),
      close: z.boolean().optional().describe("handoff: the task is finished"),
    },
    async ({ projectDir: dir, action, name, content, mode, path, query, includeOld, task, title, decision, why, supersedes, text: noteText, tags, stage, done, next, questions, close }) => {
      if (action === "overview") return plain(memoryOverview(dir));
      if (action === "read" || action === "write" || action === "trash") {
        if (!name) throw new Error(`${action} needs name`);
        if (action === "read") {
          const r = readNote(dir, name);
          const links = r.links.map((l) => `${l.rel ? `${l.rel}: ` : ""}${l.kind === "note" ? "" : `${l.kind}:`}${l.target}${l.missing ? " (missing)" : ""}`);
          return plain([`# ${r.note} — ${r.path}`, "", r.text.trim(), "", links.length && `Links: ${links.join(" · ")}`, r.backlinks.length && `Linked from: ${r.backlinks.join(" · ")}`].filter(Boolean).join("\n"));
        }
        if (action === "write") return text(writeBrainNote(dir, name, { text: content, mode, tags }));
        return text(trashNote(dir, name));
      }
      if (action === "restore") {
        if (!path) throw new Error("restore needs path (from trash)");
        return text(restoreNote(dir, path));
      }
      if (action === "find") {
        if (!query) throw new Error("find needs query");
        return plain(memoryFind(dir, query, { includeOld }));
      }
      if (action === "decide") {
        if (!title || !decision) throw new Error("decide needs title and decision");
        return plain(memoryDecide(dir, { title, decision, why, tags, supersedes }));
      }
      if (action === "note") {
        if (!noteText) throw new Error("note needs text");
        return plain(memoryNote(dir, { task, text: noteText, tags }));
      }
      if (!task) throw new Error("handoff needs task");
      return plain(memoryHandoff(dir, { task, stage, done, next, questions, close }));
    },
  );

  // ---- design graph ----

  const GRAPH_TIMEOUT_MS = 120_000;
  const graphTool = (name, description, inputSchema, render = (v) => text(v)) =>
    tool(name, description, inputSchema, async (args) => render(await send(name, args, GRAPH_TIMEOUT_MS), args));

  graphTool(
    "design_map",
    "Compact overview of the whole file: pages, Prototype screens only (size, review status from their section name: draft / review / approved, components used, where they link to); Slides / Storyboard / Graphic sections are listed as other work, flows, breakpoint sets, component usage and unused components. Cached and refreshed automatically when the file changes. Start here instead of reading screens one by one.",
    {
      pageId: z.string().optional().describe("Limit to one page (id or name)"),
      refresh: z.boolean().default(false),
      format: z.enum(["text", "json"]).default("text"),
      maxScreens: z.number().int().min(1).max(1000).default(150).describe("Per page, in text format"),
    },
    (map, { format, maxScreens }) => (format === "json" ? text(map) : { content: [{ type: "text", text: renderDesignMap(map, maxScreens) }] }),
  );

  graphTool(
    "component_usage",
    "Where components are used: pass component (name, partial ok) → screens + instance ids per variant; pass screenId → components on that screen; neither → all components by usage.",
    { component: z.string().optional(), screenId: z.string().optional().describe("Screen id or name"), pageId: z.string().optional() },
  );

  graphTool(
    "find_duplicates",
    "Find layers that should be one component: identical structures drawn separately (duplicate) and near-copies with different spacing/radius/font sizes (inconsistent). Proposes only — ask the user before converting.",
    {
      scopeId: z.string().optional().describe("Page or frame to scan; defaults to the current page"),
      minCount: z.number().int().min(2).default(2),
      limit: z.number().int().min(1).max(50).default(15),
    },
  );

  graphTool(
    "check_flows",
    "Check prototype flows: dead ends (reached but no way out), orphan screens, screens unreachable from flow starts, broken links, and buttons/tabs/links without any interaction.",
    { pageId: z.string().optional(), limit: z.number().int().min(1).max(200).default(30) },
  );

  // ---- shapes ----

  const shapeBase = {
    x: z.number().optional(),
    y: z.number().optional(),
    width: z.number().positive().default(100),
    height: z.number().positive().default(100),
    parentId,
    fill: hex.optional(),
    ...sizingProps,
  };

  tool("create_polygon", "Create a regular polygon (3 = triangle).", {
    name: z.string().default("Polygon"),
    pointCount: z.number().int().min(3).max(60).default(3),
    ...shapeBase,
  });

  tool("create_star", "Create a star.", {
    name: z.string().default("Star"),
    pointCount: z.number().int().min(3).max(60).default(5),
    innerRadius: z.number().min(0).max(1).default(0.382).describe("0–1, how deep the points are"),
    ...shapeBase,
  });

  tool("create_line", "Create a straight line (dividers, connectors). Rotation in degrees.", {
    name: z.string().default("Line"),
    x: z.number().optional(),
    y: z.number().optional(),
    length: z.number().positive().default(100),
    rotation: z.number().optional(),
    color: hex.default("#000000"),
    weight: px.default(1),
    cap: z.enum(["NONE", "ROUND", "SQUARE", "ARROW_LINES", "ARROW_EQUILATERAL"]).optional(),
    parentId,
    ...sizingProps,
  });

  tool(
    "create_from_svg",
    "Import SVG markup (or a local .svg file) as a frame of vectors — the right way to add icons. `color` recolors the icon's shapes (solid fills/strokes, keeping their opacity); the wrapping frame's background is left as is.",
    {
      svg: z.string().optional().describe("SVG markup starting with <svg"),
      file: z.string().optional().describe("Absolute path to a local .svg file"),
      name: z.string().default("Icon"),
      x: z.number().optional(),
      y: z.number().optional(),
      width: z.number().positive().optional().describe("Scale proportionally to this width"),
      color: hex.optional(),
      parentId,
      ...sizingProps,
    },
    async ({ svg, file, ...rest }) => {
      const markup = svg ?? (file ? await readFile(file, "utf8") : undefined);
      if (!markup) throw new Error("Pass svg (markup) or file (path to .svg)");
      if (markup.length > 1_000_000) throw new Error("SVG is larger than 1 MB");
      if (!/<svg[\s>]/i.test(markup)) throw new Error("Not SVG markup — expected an <svg> element");
      return text(await send("create_from_svg", { ...rest, svg: markup }));
    },
  );

  tool(
    "boolean_shapes",
    "Combine sibling layers with a boolean operation (editable afterwards, like Figma's toolbar). For SUBTRACT the bottom-most layer is the base.",
    {
      nodeIds: z.array(z.string()).min(2),
      operation: z.enum(["UNION", "SUBTRACT", "INTERSECT", "EXCLUDE"]),
      name: z.string().optional(),
    },
  );

  tool("flatten", "Flatten sibling layers into one vector (not reversible — prefer boolean_shapes when editing may be needed).", {
    nodeIds,
    name: z.string().optional(),
  });

  tool("outline_stroke", "Convert a layer's stroke into a filled vector shape (placed above the original).", {
    nodeId,
    removeOriginal: z.boolean().default(false),
    name: z.string().optional(),
  });

  tool(
    "use_as_mask",
    "Use a layer as a mask for sibling layers (like right-click → Use as mask). Groups them with the mask at the bottom.",
    {
      maskNodeId: z.string().describe("The shape that defines the visible area"),
      nodeIds: nodeIds.describe("Layers to be clipped by the mask (same parent as the mask)"),
      maskType: z.enum(["ALPHA", "VECTOR", "LUMINANCE"]).default("ALPHA").describe("ALPHA = by opacity, VECTOR = by shape outline, LUMINANCE = by brightness"),
      name: z.string().optional(),
    },
  );

  tool("remove_mask", "Turn a mask layer back into a normal layer.", { nodeId });

  // ---- prototype ----

  tool(
    "get_prototype",
    "Read prototype flows (starting points) and every interaction on the current page, or inside `nodeId`.",
    { nodeId: z.string().optional(), limit: z.number().int().min(1).max(500).default(50) },
  );

  tool(
    "add_interaction",
    "Add a prototype interaction (the blue connector): a trigger on `nodeId` that navigates, opens an overlay, goes back, etc.",
    {
      nodeId: nodeId.describe("The layer the user interacts with (button, tab, card…)"),
      trigger: z
        .enum(["ON_CLICK", "ON_HOVER", "ON_PRESS", "ON_DRAG", "AFTER_TIMEOUT", "MOUSE_ENTER", "MOUSE_LEAVE", "MOUSE_DOWN", "MOUSE_UP"])
        .default("ON_CLICK")
        .describe("AFTER_TIMEOUT only works on top-level frames"),
      delay: z.number().min(0).optional().describe("Seconds: the timeout for AFTER_TIMEOUT, or delay for MOUSE_* triggers"),
      action: z
        .enum(["NAVIGATE", "OVERLAY", "SWAP", "SCROLL_TO", "CHANGE_TO", "BACK", "CLOSE", "URL"])
        .default("NAVIGATE")
        .describe("SWAP = swap overlay, CHANGE_TO = switch component variant, CLOSE = close overlay"),
      destinationId: z.string().optional().describe("Target frame/layer; required except for BACK, CLOSE and URL"),
      url: z.string().url().optional().describe("Required for URL"),
      transition: z
        .enum(["INSTANT", "DISSOLVE", "SMART_ANIMATE", "MOVE_IN", "MOVE_OUT", "PUSH", "SLIDE_IN", "SLIDE_OUT"])
        .default("SMART_ANIMATE"),
      direction: z.enum(["LEFT", "RIGHT", "TOP", "BOTTOM"]).default("LEFT").describe("For MOVE/PUSH/SLIDE: the side it moves toward"),
      duration: z.number().min(0).max(10).default(0.3).describe("Seconds"),
      easing: z.enum(["EASE_OUT", "EASE_IN", "EASE_IN_AND_OUT", "LINEAR"]).default("EASE_OUT"),
      preserveScroll: z.boolean().default(false),
      replace: z.boolean().default(true).describe("Replace an existing interaction with the same trigger on this node"),
    },
    async (args) => {
      const needsTarget = !["BACK", "CLOSE", "URL"].includes(args.action);
      if (needsTarget && !args.destinationId) throw new Error(`action ${args.action} needs destinationId`);
      if (args.action === "URL" && !args.url) throw new Error("action URL needs url");
      return text(await send("add_interaction", args));
    },
  );

  tool("remove_interactions", "Remove all interactions from a node, or only those with the given trigger.", {
    nodeId,
    trigger: z.string().optional(),
  });

  tool(
    "set_flow_start",
    "Mark a top-level frame as a prototype flow starting point (or remove it). The first flow is the default when presenting.",
    { nodeId, name: z.string().optional().describe("Flow name, e.g. Onboarding"), remove: z.boolean().default(false) },
  );

  tool(
    "set_scroll",
    "Set a frame's scroll behavior in the prototype and which direct children stay fixed (header, tab bar). Fixed layers are moved on top; inside auto layout, make them absolute (set_props) first.",
    {
      nodeId,
      overflow: z.enum(["NONE", "HORIZONTAL", "VERTICAL", "BOTH"]).optional(),
      fixedNodeIds: z.array(z.string()).optional().describe("Direct children to keep fixed; [] clears"),
    },
  );

  // ---- design system ----

  tool("apply_style", "Apply local style ids (from get_styles) to a node.", {
    nodeId,
    fillStyleId: z.string().optional(),
    strokeStyleId: z.string().optional(),
    textStyleId: z.string().optional(),
    effectStyleId: z.string().optional(),
  });

  tool("bind_variable", "Bind a variable (from get_variables) to a node field; fill/stroke bind the color.", {
    nodeId,
    field: z.enum(BINDABLE_FIELDS),
    variableId: z.string(),
  });

  const variableValue = z.union([z.string(), z.number(), z.boolean(), z.object({ alias: z.string().describe("Variable id or name") })]);
  const variableType = z.enum(["COLOR", "FLOAT", "STRING", "BOOLEAN"]);

  tool(
    "create_variable_collection",
    'Create a variable collection with modes, e.g. {name: "Color", modes: ["Light", "Dark"]} (first mode is the default). The Figma plan may limit the number of modes.',
    { name: z.string().min(1), modes: z.array(z.string().min(1)).min(1).max(10).default(["Default"]) },
  );

  tool(
    "create_variables",
    'Create many variables in one call. values are per mode name: hex for COLOR, number for FLOAT, or {alias: "Color/Primary/500"} to reference another variable. Slash names group them (Color/Primary/500, Spacing/4).',
    {
      collectionId: z.string(),
      variables: z
        .array(
          z.object({
            name: z.string().min(1),
            type: variableType,
            values: z.record(z.string(), variableValue),
            scopes: z
              .array(z.string())
              .optional()
              .describe("Where it can be used, e.g. ALL_FILLS, TEXT_FILL, STROKE_COLOR, GAP, CORNER_RADIUS, FONT_SIZE"),
          }),
        )
        .min(1)
        .max(500),
    },
  );

  tool("set_variable_value", "Change a variable's value for one mode.", {
    variableId: z.string(),
    mode: z.string().describe("Mode name or id"),
    value: variableValue,
  });

  tool("set_instance_properties", "Set component properties on an instance (names as shown by get_node).", {
    nodeId,
    properties: z.record(z.string(), z.union([z.string(), z.boolean()])),
  });

  return tools;
}
