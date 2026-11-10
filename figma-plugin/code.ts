// Light-only window matching the "Plugin/Window" frame in the wara-bridge Figma file.
figma.showUI(__html__, { width: 300, height: 236, themeColors: false });
// The version goes up with wridge; fileKey (private plugin API) lets the wridge window remember a file across renames.
const PLUGIN_VERSION = '1.8.0';
figma.ui.postMessage({ type: 'hello', fileName: figma.root.name, editorType: figma.editorType, fileKey: figma.fileKey ?? null, version: PLUGIN_VERSION });

type Command = { id: string; command: string; params: Record<string, unknown> };
// Messages from the plugin window itself (not commands relayed from the server).
type UiMessage = { type: 'open'; url: string } | { type: 'close' } | { type: 'resize'; height: number };
type P = Record<string, unknown>;
type AutoLayoutNode = Extract<SceneNode, { layoutMode: string }>;

const num = (v: unknown) => v as number;
const str = (v: unknown) => v as string;
const has = (p: P, key: string) => p[key] !== undefined && p[key] !== null;
const round = (n: number) => Math.round(n * 100) / 100;

// ---------- colors ----------

function parseHex(hex: string): { color: RGB; opacity: number } {
  const h = hex.slice(1);
  const channel = (i: number) => parseInt(h.slice(i, i + 2), 16) / 255;
  return { color: { r: channel(0), g: channel(2), b: channel(4) }, opacity: h.length === 8 ? channel(6) : 1 };
}

function solid(hex: string): SolidPaint {
  const { color, opacity } = parseHex(hex);
  return { type: 'SOLID', color, opacity };
}

function rgba(hex: string): RGBA {
  const { color, opacity } = parseHex(hex);
  return { ...color, a: opacity };
}

function toHex(color: RGB, opacity = 1): string {
  const part = (v: number) =>
    Math.round(v * 255)
      .toString(16)
      .padStart(2, '0');
  return `#${part(color.r)}${part(color.g)}${part(color.b)}${opacity < 1 ? part(opacity) : ''}`.toUpperCase();
}

function solidHexes(paints: unknown): string[] | undefined {
  if (!Array.isArray(paints)) return undefined;
  const hexes = (paints as readonly Paint[])
    .filter((p): p is SolidPaint => p.type === 'SOLID' && p.visible !== false)
    .map((p) => toHex(p.color, p.opacity));
  return hexes.length ? hexes : undefined;
}

// ---------- reading ----------

// Figma's rotation turns a node around its top-left corner; the canvas handle (and people) turn it
// around its centre. Set the angle, then move the node back so its centre stays where it was.
function rotateAroundCenter(node: SceneNode & LayoutMixin, degrees: number) {
  const centre = () => {
    const [[a, b, tx], [c, d, ty]] = node.relativeTransform;
    return [a * node.width / 2 + b * node.height / 2 + tx, c * node.width / 2 + d * node.height / 2 + ty];
  };
  const [x0, y0] = centre();
  node.rotation = degrees;
  const [x1, y1] = centre();
  const [[a, b, tx], [c, d, ty]] = node.relativeTransform;
  node.relativeTransform = [[a, b, tx + x0 - x1], [c, d, ty + y0 - y1]];
}

// ---- Figma Motion helpers (beta API: everything Motion-specific stays in this block) ----
function motionApi(): MotionAPI {
  const m = (figma as unknown as { motion?: MotionAPI }).motion;
  if (!m) throw new Error('Figma Motion is not available here (figma.motion is missing): it may not be enabled for this account or file yet, or Figma changed the beta API.');
  return m;
}
// A keyframe value as plain JSON: numbers, #RRGGBBAA, [x, y], text or booleans.
function motionValue(v: KeyframeValue | undefined): unknown {
  if (!v) return null;
  if (v.type === 'FLOAT' || v.type === 'TEXT_DATA' || v.type === 'BOOL') return v.value;
  if (v.type === 'COLOR') { const h = (n: number) => Math.round(n * 255).toString(16).padStart(2, '0'); return `#${h(v.value.r)}${h(v.value.g)}${h(v.value.b)}${h(v.value.a)}`.toUpperCase(); }
  if (v.type === 'VECTOR') return [round(v.value.x), round(v.value.y)];
  return v;
}
function motionEasing(e: MotionEasing | VariableAlias | undefined): unknown {
  if (!e) return undefined;
  if ('type' in e && e.type === 'VARIABLE_ALIAS') return { variable: (e as VariableAlias).id };
  const m = e as MotionEasing;
  if (m.type === 'CUSTOM_CUBIC_BEZIER' && m.easingFunctionCubicBezier) { const b = m.easingFunctionCubicBezier; return `cubic(${round(b.x1)},${round(b.y1)},${round(b.x2)},${round(b.y2)})`; }
  if (m.type === 'CUSTOM_SPRING' && m.easingFunctionSpring) return { spring: m.easingFunctionSpring };
  return m.type;
}
// What the layer is, so motion_check can say code or Lottie: text and UI components animate in code,
// pure vector art animates as a Lottie; a component named "Lottie/…" is always a Lottie.
// (async: with documentAccess "dynamic-page" an instance's main component can only be read with getMainComponentAsync)
async function motionHint(n: SceneNode) {
  const inner = 'findAll' in n ? (n as ChildrenMixin).findAll(() => true) : [];
  const all = [n, ...inner];
  const comp = n.type === 'INSTANCE' ? await (n as InstanceNode).getMainComponentAsync() : n.type === 'COMPONENT' ? n : null;
  const compName = comp ? (comp.parent?.type === 'COMPONENT_SET' ? comp.parent.name : comp.name) : null;
  return {
    component: compName,
    text: all.some((c) => c.type === 'TEXT'),
    instances: inner.filter((c) => c.type === 'INSTANCE').length,
    autoLayout: 'layoutMode' in n && (n as FrameNode).layoutMode !== 'NONE',
    vectorOnly: all.every((c) => ['VECTOR', 'ELLIPSE', 'RECTANGLE', 'POLYGON', 'STAR', 'LINE', 'BOOLEAN_OPERATION', 'GROUP'].includes(c.type)),
    image: all.some((c) => 'fills' in c && Array.isArray((c as GeometryMixin).fills) && ((c as GeometryMixin).fills as Paint[]).some((f) => f.type === 'IMAGE')),
    size: [Math.round(n.width), Math.round(n.height)],
  };
}
type AnyBinding = { baseValue: KeyframeValue; timelineDuration: number; tracks: ReadonlyArray<ManualKeyframeTrack> };
// Every animated field: property name (or fills[0] / effects[0].RADIUS), base value and keyframes in seconds.
function readTracks(animations: Animations) {
  const out: unknown[] = [];
  const add = (field: string, b: AnyBinding | undefined) => {
    if (!b || !b.tracks?.length) return;
    out.push({
      field, base: motionValue(b.baseValue), duration: b.timelineDuration,
      tracks: b.tracks.map((t) => ({ op: t.keyframeOperation, keys: t.keyframes.map((k) => ({ t: round(k.timelinePosition), v: motionValue(k.value), ease: motionEasing(k.easing) })) })),
    });
  };
  for (const [name, b] of Object.entries(animations)) {
    if (name === 'fills' || name === 'strokes') for (const [i, pb] of Object.entries((b ?? {}) as Record<string, unknown>)) add(`${name}[${i}]`, pb as AnyBinding);
    else if (name === 'effects') for (const [i, eb] of Object.entries((b ?? {}) as Record<string, Record<string, AnyBinding>>)) for (const [f, fb] of Object.entries(eb ?? {})) add(`effects[${i}].${f}`, fb);
    else add(name, b as AnyBinding);
  }
  return out;
}

// ---- FigJam helpers ----
type JamNode = { id?: string; kind?: 'shape' | 'sticky'; shape?: ShapeWithTextNode['shapeType']; text: string; x: number; y: number; width?: number; height?: number; fill?: string; stroke?: string; textColor?: string; fontSize?: number; bold?: boolean; role?: string; name?: string };
type JamEdge = { id?: string; from: string; to: string; label?: string; dashed?: boolean; color?: string; lineType?: ConnectorNode['connectorLineType']; fromPoint?: Vector; toPoint?: Vector; noArrow?: boolean };
type JamLane = { name: string; x: number; y: number; width: number; height: number; fill?: string };

function requireFigJam(tool: string) {
  if (figma.editorType !== 'figjam') throw new Error(`${tool} works on a FigJam board; the open file "${figma.root.name}" is a Figma design file. Switch with use_file.`);
}
// FigJam's default font has no Thai: use IBM Plex Sans Thai (falls back to Inter when it is unavailable).
const JAM_FONT = 'IBM Plex Sans Thai';
async function setJamText(t: TextSublayerNode | TextNode, text: string, o: { size?: number; bold?: boolean; color?: string } = {}) {
  const style = o.bold ? 'SemiBold' : 'Regular';
  let font: FontName = { family: JAM_FONT, style };
  try { await figma.loadFontAsync(font); } catch { font = { family: 'Inter', style: o.bold ? 'Semi Bold' : 'Regular' }; await figma.loadFontAsync(font); }
  await figma.loadFontAsync(t.fontName as FontName).catch(() => {});
  t.fontName = font;
  t.characters = text;
  if (o.size) t.fontSize = o.size;
  if (o.color) t.fills = [{ type: 'SOLID', color: parseHex(o.color).color }];
}
async function jamShape(n: JamNode) {
  const node = figma.createShapeWithText();
  node.shapeType = n.shape ?? 'ROUNDED_RECTANGLE';
  if (n.width && n.height) node.resize(n.width, n.height);
  if (n.fill) node.fills = [{ type: 'SOLID', color: parseHex(n.fill).color }];
  if (n.stroke) node.strokes = [{ type: 'SOLID', color: parseHex(n.stroke).color }];
  await setJamText(node.text, n.text, { size: n.fontSize ?? 14, bold: n.bold, color: n.textColor });
  return node;
}
// Ends are nodes (the arrow follows them) or fixed points (sequence lifelines and messages).
// Points are in the parent's space, so the connector goes into its parent before they are set.
async function jamConnector(from: string | null, to: string | null, e: Partial<JamEdge>, parent?: BaseNode & ChildrenMixin) {
  const c = figma.createConnector();
  if (parent) parent.appendChild(c);
  // A fixed end point is read in page coordinates: shift it by where the parent (a section) sits.
  const origin = parent && 'absoluteTransform' in parent ? { x: (parent as SceneNode).absoluteTransform[0][2], y: (parent as SceneNode).absoluteTransform[1][2] } : { x: 0, y: 0 };
  const end = async (id: string | null, point: Vector | undefined): Promise<ConnectorEndpoint> => {
    if (point) return { position: { x: point.x + origin.x, y: point.y + origin.y } };
    const n = id ? await figma.getNodeByIdAsync(id) : null;
    if (!n) throw new Error(`Connector end not found: ${id}`);
    return { endpointNodeId: n.id, magnet: 'AUTO' };
  };
  c.connectorStart = await end(from, e.fromPoint);
  c.connectorEnd = await end(to, e.toPoint);
  c.connectorLineType = e.lineType ?? 'ELBOWED';
  c.connectorEndStrokeCap = e.noArrow ? 'NONE' : 'ARROW_LINES';
  if (e.color) c.strokes = [{ type: 'SOLID', color: parseHex(e.color).color }];
  if (e.dashed) c.dashPattern = [8, 8];
  if (e.label) await setJamText(c.text, e.label, { size: 12 });
  return c;
}
async function placeJam(node: SceneNode, p: P) {
  if (has(p, 'parentId')) (await getParent(p.parentId)).appendChild(node);
  if (has(p, 'x')) node.x = num(p.x);
  if (has(p, 'y')) node.y = num(p.y);
  if (has(p, 'name')) node.name = str(p.name);
  markCreated(node);
}
function hexOf(fills: readonly Paint[] | PluginAPI['mixed']): string | undefined {
  if (!Array.isArray(fills)) return undefined;
  const s = (fills as readonly Paint[]).find((f) => f.type === 'SOLID') as SolidPaint | undefined;
  if (!s) return undefined;
  const h = (v: number) => Math.round(v * 255).toString(16).padStart(2, '0');
  return `#${h(s.color.r)}${h(s.color.g)}${h(s.color.b)}`.toUpperCase();
}

function serialize(node: BaseNode, depth: number): Record<string, unknown> {
  const out: Record<string, unknown> = { id: node.id, name: node.name, type: node.type };
  if ('x' in node) {
    out.x = round(node.x);
    out.y = round(node.y);
  }
  if ('width' in node) {
    out.width = round(node.width);
    out.height = round(node.height);
  }
  if ('visible' in node && !node.visible) out.visible = false;
  if ('isMask' in node && node.isMask) out.mask = node.maskType;
  if (node.type === 'BOOLEAN_OPERATION') out.booleanOperation = node.booleanOperation;
  if ('opacity' in node && node.opacity < 1) out.opacity = round(node.opacity);
  if ('rotation' in node && Math.abs(node.rotation) > 0.01) out.rotation = round(node.rotation);
  if ('layoutPositioning' in node && node.layoutPositioning === 'ABSOLUTE') out.absolute = true;
  if ('layoutSizingHorizontal' in node && node.parent && 'layoutMode' in node.parent && node.parent.layoutMode !== 'NONE') {
    out.sizing = { width: node.layoutSizingHorizontal, height: node.layoutSizingVertical };
  }

  if ('fills' in node) {
    const fills = solidHexes(node.fills);
    if (fills) out.fills = fills;
    if (Array.isArray(node.fills)) {
      const images = (node.fills as readonly Paint[])
        .filter((p): p is ImagePaint => p.type === 'IMAGE' && p.visible !== false)
        .map((p) => ({ scaleMode: p.scaleMode }));
      if (images.length) out.images = images;
    }
    if (typeof node.fillStyleId === 'string' && node.fillStyleId) out.fillStyleId = node.fillStyleId;
  }
  if ('strokes' in node && node.strokes.length) {
    out.strokes = solidHexes(node.strokes);
    if (typeof node.strokeWeight === 'number') out.strokeWeight = node.strokeWeight;
  }
  if ('effects' in node && node.effects.length) out.effects = node.effects.map((e) => e.type);
  if ('cornerRadius' in node && node.cornerRadius !== 0) {
    out.cornerRadius = node.cornerRadius === figma.mixed ? 'mixed' : node.cornerRadius;
  }
  if ('boundVariables' in node && node.boundVariables && Object.keys(node.boundVariables).length) {
    out.boundVariables = Object.keys(node.boundVariables);
  }

  if (node.type === 'TEXT') {
    out.characters = node.characters;
    out.fontName = node.fontName === figma.mixed ? 'mixed' : node.fontName;
    out.fontSize = node.fontSize === figma.mixed ? 'mixed' : node.fontSize;
    out.lineHeight = node.lineHeight === figma.mixed ? 'mixed' : node.lineHeight;
    out.textAlign = node.textAlignHorizontal;
    out.autoResize = node.textAutoResize;
    if (typeof node.textStyleId === 'string' && node.textStyleId) out.textStyleId = node.textStyleId;
  }

  if ('layoutMode' in node && node.layoutMode !== 'NONE') {
    out.autoLayout = {
      mode: node.layoutMode,
      itemSpacing: node.itemSpacing,
      padding: [node.paddingTop, node.paddingRight, node.paddingBottom, node.paddingLeft],
      primaryAlign: node.primaryAxisAlignItems,
      counterAlign: node.counterAxisAlignItems,
      wrap: node.layoutWrap === 'WRAP',
    };
  }

  if (node.type === 'INSTANCE') {
    out.componentProperties = Object.fromEntries(
      Object.entries(node.componentProperties).map(([name, prop]) => [name, prop.value]),
    );
  }
  if (node.type === 'COMPONENT' || node.type === 'COMPONENT_SET') out.key = node.key;

  if ('reactions' in node && node.reactions.length) out.interactions = node.reactions.length;
  if ('overflowDirection' in node) {
    if (node.overflowDirection !== 'NONE') out.overflow = node.overflowDirection;
    if (node.numberOfFixedChildren > 0) out.fixedChildren = node.numberOfFixedChildren;
  }

  if ('children' in node) {
    if (depth > 0) out.children = node.children.map((child) => serialize(child, depth - 1));
    else out.childCount = node.children.length;
  }
  return out;
}

async function getNode(id: unknown): Promise<BaseNode> {
  const node = await figma.getNodeByIdAsync(String(id));
  if (!node) throw new Error(`Node ${id} not found`);
  return node;
}

async function getSceneNode(id: unknown): Promise<SceneNode> {
  const node = await getNode(id);
  if (node.type === 'DOCUMENT' || node.type === 'PAGE') throw new Error(`Node ${id} is a ${node.type}, not a layer`);
  return node;
}

async function getParent(id: unknown): Promise<BaseNode & ChildrenMixin> {
  if (id === undefined || id === null) return figma.currentPage;
  const node = await getNode(id);
  if (node.type === 'DOCUMENT' || !('appendChild' in node)) {
    throw new Error(`Node ${id} (${node.type}) cannot contain layers`);
  }
  return node;
}

async function getAutoLayoutNode(id: unknown): Promise<AutoLayoutNode> {
  const node = await getSceneNode(id);
  if (!('layoutMode' in node)) throw new Error(`Node ${id} (${node.type}) cannot have auto layout — use a frame`);
  return node;
}

async function getText(id: unknown): Promise<TextNode> {
  const node = await getNode(id);
  if (node.type !== 'TEXT') throw new Error(`Node ${id} is ${node.type}, not TEXT`);
  return node;
}

// ---------- layout helpers ----------

function applyAutoLayout(node: AutoLayoutNode, p: P) {
  if (has(p, 'layoutMode')) node.layoutMode = p.layoutMode as AutoLayoutNode['layoutMode'];
  if (node.layoutMode === 'NONE') return;
  if (has(p, 'itemSpacing')) node.itemSpacing = num(p.itemSpacing);
  if (has(p, 'counterAxisSpacing')) node.counterAxisSpacing = num(p.counterAxisSpacing);
  if (has(p, 'wrap')) node.layoutWrap = p.wrap ? 'WRAP' : 'NO_WRAP';
  if (has(p, 'padding')) node.paddingTop = node.paddingRight = node.paddingBottom = node.paddingLeft = num(p.padding);
  if (has(p, 'paddingX')) node.paddingLeft = node.paddingRight = num(p.paddingX);
  if (has(p, 'paddingY')) node.paddingTop = node.paddingBottom = num(p.paddingY);
  if (has(p, 'paddingTop')) node.paddingTop = num(p.paddingTop);
  if (has(p, 'paddingRight')) node.paddingRight = num(p.paddingRight);
  if (has(p, 'paddingBottom')) node.paddingBottom = num(p.paddingBottom);
  if (has(p, 'paddingLeft')) node.paddingLeft = num(p.paddingLeft);
  if (has(p, 'primaryAlign')) node.primaryAxisAlignItems = p.primaryAlign as AutoLayoutNode['primaryAxisAlignItems'];
  if (has(p, 'counterAlign')) node.counterAxisAlignItems = p.counterAlign as AutoLayoutNode['counterAxisAlignItems'];
}

type Sizing = 'FIXED' | 'HUG' | 'FILL';

function applySizing(node: SceneNode, p: P) {
  if (!('layoutSizingHorizontal' in node)) return;
  if (has(p, 'widthSizing')) node.layoutSizingHorizontal = p.widthSizing as Sizing;
  if (has(p, 'heightSizing')) node.layoutSizingVertical = p.heightSizing as Sizing;
  if ('minWidth' in node) {
    if (has(p, 'minWidth')) node.minWidth = num(p.minWidth);
    if (has(p, 'maxWidth')) node.maxWidth = num(p.maxWidth);
    if (has(p, 'minHeight')) node.minHeight = num(p.minHeight);
    if (has(p, 'maxHeight')) node.maxHeight = num(p.maxHeight);
  }
}

function applyCorners(node: SceneNode, p: P) {
  if (!('cornerRadius' in node)) return;
  if (has(p, 'cornerRadius')) (node as SceneNode & CornerMixin).cornerRadius = num(p.cornerRadius);
  if ('topLeftRadius' in node) {
    if (has(p, 'topLeft')) node.topLeftRadius = num(p.topLeft);
    if (has(p, 'topRight')) node.topRightRadius = num(p.topRight);
    if (has(p, 'bottomRight')) node.bottomRightRadius = num(p.bottomRight);
    if (has(p, 'bottomLeft')) node.bottomLeftRadius = num(p.bottomLeft);
  }
}

// ---------- vectors: Figma's vector edit mode (Enter), without the mouse ----------

type Pt = { x: number; y: number };
type NV = { x: number; y: number; strokeCap?: StrokeCap; strokeJoin?: StrokeJoin; cornerRadius?: number; handleMirroring?: HandleMirroring };
type NS = { start: number; end: number; tangentStart?: Pt; tangentEnd?: Pt };
type NR = { windingRule: WindingRule; loops: number[][]; fills?: Paint[]; fillStyleId?: string };
type Net = { vertices: NV[]; segments: NS[]; regions: NR[] };

const TO_VECTOR = new Set(['RECTANGLE', 'ELLIPSE', 'POLYGON', 'STAR', 'LINE']);
const r2 = (n: number) => Math.round(n * 100) / 100;
const add = (a: Pt, b: Pt): Pt => ({ x: a.x + b.x, y: a.y + b.y });
const sub = (a: Pt, b: Pt): Pt => ({ x: a.x - b.x, y: a.y - b.y });
const mul = (a: Pt, k: number): Pt => ({ x: a.x * k, y: a.y * k });
const lerp = (a: Pt, b: Pt, t: number): Pt => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t });
const len = (a: Pt) => Math.hypot(a.x, a.y);
const isCurved = (s: NS) => !!((s.tangentStart && len(s.tangentStart) > 1e-6) || (s.tangentEnd && len(s.tangentEnd) > 1e-6));

function netOf(node: VectorNode): Net {
  const n = JSON.parse(JSON.stringify(node.vectorNetwork)) as Net;
  n.regions = n.regions ?? [];
  return n;
}

// A rectangle / ellipse / polygon / star / line turns into a vector (what Enter does in Figma). Same place, name and owner.
async function editableVector(id: unknown): Promise<VectorNode> {
  const node = await getSceneNode(id);
  if (node.type === 'VECTOR') return node;
  if (!TO_VECTOR.has(node.type)) throw new Error(`${node.name} (${node.type}) has no points to edit — vectors, rectangles, ellipses, polygons, stars and lines do (a boolean group: flatten it first)`);
  const parent = node.parent;
  if (!parent) throw new Error(`Node ${node.id} has no parent`);
  const mine = createdByWara(node);
  const name = node.name;
  const vector = figma.flatten([node], parent, parent.children.indexOf(node));
  vector.name = name;
  if (mine) markCreated(vector);
  return vector;
}

function boundsOf(net: Net) {
  const xs = net.vertices.map((v) => v.x), ys = net.vertices.map((v) => v.y);
  const b = { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) };
  return { ...b, eps: Math.max(0.5, Math.max(b.maxX - b.minX, b.maxY - b.minY) * 0.01) };
}
const SIDES = ['top', 'bottom', 'left', 'right'] as const;
const CORNERS = ['top-left', 'top-right', 'bottom-right', 'bottom-left'] as const;
function onSide(net: Net, i: number, side: string) {
  const b = boundsOf(net), v = net.vertices[i];
  if (side === 'top') return Math.abs(v.y - b.minY) <= b.eps;
  if (side === 'bottom') return Math.abs(v.y - b.maxY) <= b.eps;
  if (side === 'left') return Math.abs(v.x - b.minX) <= b.eps;
  return Math.abs(v.x - b.maxX) <= b.eps;
}
function cornerPoint(net: Net, corner: string) {
  const b = boundsOf(net);
  const cx = corner.endsWith('left') ? b.minX : b.maxX, cy = corner.startsWith('top') ? b.minY : b.maxY;
  let best = 0, d = Infinity;
  net.vertices.forEach((v, i) => { const e = Math.hypot(v.x - cx, v.y - cy); if (e < d) { d = e; best = i; } });
  return best;
}
const sidePoints = (net: Net, side: string) => net.vertices.map((_, i) => i).filter((i) => onSide(net, i, side));
const sideSegments = (net: Net, side: string) => net.segments.map((_, i) => i).filter((i) => onSide(net, net.segments[i].start, side) && onSide(net, net.segments[i].end, side));

function pick(net: Net, sel: unknown, kind: 'points' | 'segments'): number[] {
  const count = kind === 'points' ? net.vertices.length : net.segments.length;
  const one = (s: unknown): number[] => {
    if (typeof s === 'number') {
      if (!Number.isInteger(s) || s < 0 || s >= count) throw new Error(`No ${kind === 'points' ? 'point' : 'segment'} ${s} — this shape has ${count} (0–${count - 1}); vector_read lists them`);
      return [s];
    }
    const k = String(s).toLowerCase().trim();
    if (k === 'all') return Array.from({ length: count }, (_, i) => i);
    if ((SIDES as readonly string[]).includes(k)) {
      const got = kind === 'points' ? sidePoints(net, k) : sideSegments(net, k);
      if (!got.length) throw new Error(`No ${kind === 'points' ? 'point' : 'straight edge'} lies on the ${k} side — vector_read shows the sides`);
      return got;
    }
    if ((CORNERS as readonly string[]).includes(k)) {
      if (kind === 'points') return [cornerPoint(net, k)];
      throw new Error(`"${k}" is a corner (a point), not a segment`);
    }
    throw new Error(`Unknown ${kind} "${String(s)}": use an index, a list, top / bottom / left / right, a corner like top-left, or all`);
  };
  const list = Array.isArray(sel) ? sel.flatMap(one) : one(sel);
  return [...new Set(list)];
}

const center = (net: Net) => mul(net.vertices.reduce((a, v) => add(a, v), { x: 0, y: 0 }), 1 / Math.max(1, net.vertices.length));

// Bend a segment so its middle sits `amount` px off the straight line (+ away from the shape's center).
function curveSegment(net: Net, si: number, amount: number) {
  const s = net.segments[si];
  const A = net.vertices[s.start], B = net.vertices[s.end];
  const d = sub(B, A), L = len(d);
  if (L < 1e-6) return;
  let n = { x: -d.y / L, y: d.x / L };
  const mid = lerp(A, B, 0.5), c = center(net);
  if ((mid.x - c.x) * n.x + (mid.y - c.y) * n.y < 0) n = mul(n, -1);
  const h = amount / 0.75;
  s.tangentStart = add(mul(d, 1 / 3), mul(n, h));
  s.tangentEnd = add(mul(d, -1 / 3), mul(n, h));
}

// Keep closed areas valid when segment `si` gains a neighbour `ni` that continues from its end.
function insertInLoops(net: Net, si: number, ni: number, oldEnd: number) {
  for (const r of net.regions) {
    for (const loop of r.loops) {
      const k = loop.indexOf(si);
      if (k < 0) continue;
      const next = net.segments[loop[(k + 1) % loop.length]];
      const forward = loop.length > 1 && next && (next.start === oldEnd || next.end === oldEnd);
      loop.splice(forward ? k + 1 : k, 0, ni);
    }
  }
}

// Add a point on a segment at `t` (de Casteljau, so curves keep their shape). Returns the new segment (second half).
function splitSegment(net: Net, si: number, t: number): { point: number; segment: number } {
  const s = net.segments[si];
  const P0 = net.vertices[s.start], P3 = net.vertices[s.end];
  const oldEnd = s.end;
  const vi = net.vertices.length;
  let second: NS;
  if (!isCurved(s)) {
    const M = lerp(P0, P3, t);
    net.vertices.push({ x: M.x, y: M.y });
    second = { start: vi, end: oldEnd };
    s.end = vi;
  } else {
    const P1 = add(P0, s.tangentStart ?? { x: 0, y: 0 }), P2 = add(P3, s.tangentEnd ?? { x: 0, y: 0 });
    const P01 = lerp(P0, P1, t), P12 = lerp(P1, P2, t), P23 = lerp(P2, P3, t);
    const P012 = lerp(P01, P12, t), P123 = lerp(P12, P23, t), M = lerp(P012, P123, t);
    net.vertices.push({ x: M.x, y: M.y, handleMirroring: 'ANGLE_AND_LENGTH' });
    second = { start: vi, end: oldEnd, tangentStart: sub(P123, M), tangentEnd: sub(P23, P3) };
    s.end = vi;
    s.tangentStart = sub(P01, P0);
    s.tangentEnd = sub(P012, M);
  }
  net.segments.push(second);
  const ni = net.segments.length - 1;
  insertInLoops(net, si, ni, oldEnd);
  return { point: vi, segment: ni };
}

function dropRegionsWith(net: Net, segs: Set<number>) {
  net.regions = net.regions.filter((r) => !r.loops.some((l) => l.some((i) => segs.has(i))));
}
function removeSegments(net: Net, gone: Set<number>) {
  dropRegionsWith(net, gone);
  const map: number[] = [];
  const kept: NS[] = [];
  net.segments.forEach((s, i) => { if (gone.has(i)) return; map[i] = kept.length; kept.push(s); });
  net.segments = kept;
  for (const r of net.regions) r.loops = r.loops.map((l) => l.map((i) => map[i]));
}
function removePoints(net: Net, gone: Set<number>) {
  // a point between exactly two segments: its neighbours join up (like Delete in Figma); otherwise its segments go too
  const dropSegs = new Set<number>();
  for (const vi of gone) {
    const touching = net.segments.map((s, i) => i).filter((i) => !dropSegs.has(i) && (net.segments[i].start === vi || net.segments[i].end === vi));
    if (touching.length === 2) {
      const [a, b] = touching;
      const sa = net.segments[a], sb = net.segments[b];
      // the far end of each segment, with its handle there
      const far = (s: NS) => (s.start === vi ? { at: s.end, tan: s.tangentEnd } : { at: s.start, tan: s.tangentStart });
      const x = far(sa), y = far(sb);
      if (gone.has(x.at) || gone.has(y.at) || x.at === y.at) { touching.forEach((i) => dropSegs.add(i)); continue; }
      sa.start = x.at; sa.tangentStart = x.tan; sa.end = y.at; sa.tangentEnd = y.tan;
      dropSegs.add(b);
      for (const r of net.regions) for (const l of r.loops) { const k = l.indexOf(b); if (k >= 0) l.splice(k, 1); }
    } else touching.forEach((i) => dropSegs.add(i));
  }
  // segments already spliced out of loops above aren't in any loop, so only real breaks drop a region
  removeSegments(net, dropSegs);
  const map: number[] = [];
  const kept: NV[] = [];
  net.vertices.forEach((v, i) => { if (gone.has(i)) return; map[i] = kept.length; kept.push(v); });
  net.vertices = kept;
  for (const s of net.segments) { s.start = map[s.start]; s.end = map[s.end]; }
}

function chamfer(net: Net, vi: number, amount: number) {
  const touching = net.segments.map((_, i) => i).filter((i) => net.segments[i].start === vi || net.segments[i].end === vi);
  if (touching.length !== 2) throw new Error(`Point ${vi} isn't a corner between two segments`);
  const [a, b] = touching;
  const V = net.vertices[vi];
  const far = (si: number) => { const s = net.segments[si]; return net.vertices[s.start === vi ? s.end : s.start]; };
  const pa = add(V, mul(sub(far(a), V), Math.min(0.49, amount / Math.max(1e-6, len(sub(far(a), V)))))),
    pb = add(V, mul(sub(far(b), V), Math.min(0.49, amount / Math.max(1e-6, len(sub(far(b), V))))));
  for (const si of [a, b]) { const s = net.segments[si]; if (s.start === vi) s.tangentStart = undefined; else s.tangentEnd = undefined; }
  V.x = pa.x; V.y = pa.y; V.cornerRadius = 0;
  const nv = net.vertices.length;
  net.vertices.push({ x: pb.x, y: pb.y });
  const sb = net.segments[b];
  if (sb.start === vi) sb.start = nv; else sb.end = nv;
  net.segments.push({ start: vi, end: nv });
  const ns = net.segments.length - 1;
  for (const r of net.regions) for (const l of r.loops) {
    const ia = l.indexOf(a), ib = l.indexOf(b);
    if (ia < 0 || ib < 0) continue;
    if ((ia + 1) % l.length === ib) l.splice(ia + 1, 0, ns);
    else l.splice(ib + 1, 0, ns);
  }
}

function describeNet(node: VectorNode, net: Net) {
  const sides: Record<string, unknown> = {};
  if (net.vertices.length) for (const s of SIDES) sides[s] = { points: sidePoints(net, s), segments: sideSegments(net, s) };
  const corners: Record<string, number> = {};
  if (net.vertices.length) for (const c of CORNERS) corners[c] = cornerPoint(net, c);
  return {
    id: node.id, name: node.name, type: node.type, width: r2(node.width), height: r2(node.height),
    points: net.vertices.map((v, i) => ({ i, x: r2(v.x), y: r2(v.y), ...(v.cornerRadius ? { corner: v.cornerRadius } : {}), ...(v.strokeCap && v.strokeCap !== 'NONE' ? { cap: v.strokeCap } : {}) })),
    segments: net.segments.map((s, i) => ({ i, from: s.start, to: s.end, ...(isCurved(s) ? { curve: { start: [r2(s.tangentStart?.x ?? 0), r2(s.tangentStart?.y ?? 0)], end: [r2(s.tangentEnd?.x ?? 0), r2(s.tangentEnd?.y ?? 0)] } } : {}) })),
    regions: net.regions.map((r, i) => ({ i, loops: r.loops, ...(r.fills?.length ? { fill: r.fills[0].type === 'SOLID' ? toHex((r.fills[0] as SolidPaint).color) : r.fills[0].type } : {}) })),
    sides, corners,
  };
}

function applyOp(net: Net, op: P) {
  const pts = () => pick(net, op.points, 'points');
  const segs = () => pick(net, op.segments ?? op.segment, 'segments');
  switch (op.op) {
    case 'move': {
      const list = pts();
      if (has(op, 'x') || has(op, 'y')) {
        if (list.length !== 1) throw new Error('move with x / y takes one point; use dx / dy for several');
        const v = net.vertices[list[0]];
        if (has(op, 'x')) v.x = num(op.x);
        if (has(op, 'y')) v.y = num(op.y);
      } else for (const i of list) { net.vertices[i].x += num(op.dx ?? 0); net.vertices[i].y += num(op.dy ?? 0); }
      return;
    }
    case 'curve': for (const i of segs()) curveSegment(net, i, num(op.amount ?? 20)); return;
    case 'handles': {
      const s = net.segments[segs()[0]];
      const v = (a: unknown) => { const t = a as number[] | undefined; return t && t.length === 2 ? { x: t[0], y: t[1] } : undefined; };
      if (op.start) s.tangentStart = v(op.start);
      if (op.end) s.tangentEnd = v(op.end);
      return;
    }
    case 'straighten': for (const i of segs()) { net.segments[i].tangentStart = undefined; net.segments[i].tangentEnd = undefined; } return;
    case 'split': for (const i of segs().sort((a, b) => b - a)) splitSegment(net, i, has(op, 'at') ? num(op.at) : 0.5); return;
    case 'cut': {
      for (const i of segs().sort((a, b) => b - a)) {
        const { point, segment } = splitSegment(net, i, has(op, 'at') ? num(op.at) : 0.5);
        const copy = { ...net.vertices[point] };
        net.vertices.push(copy);
        net.segments[segment].start = net.vertices.length - 1;
        dropRegionsWith(net, new Set([i, segment]));
      }
      return;
    }
    case 'remove': {
      if (has(op, 'points')) removePoints(net, new Set(pts()));
      if (has(op, 'segments') || has(op, 'segment')) removeSegments(net, new Set(segs()));
      return;
    }
    case 'connect': {
      const n = net.vertices.length;
      const f = num(op.from), t = num(op.to);
      if (f >= n || t >= n || f === t) throw new Error(`connect needs two different points (0–${n - 1})`);
      net.segments.push({ start: f, end: t });
      return;
    }
    case 'corner': for (const i of pts()) net.vertices[i].cornerRadius = num(op.radius ?? 0); return;
    case 'cap': for (const i of pts()) net.vertices[i].strokeCap = (op.cap ?? 'NONE') as StrokeCap; return;
    case 'mirror': for (const i of pts()) net.vertices[i].handleMirroring = (op.mode ?? 'NONE') as HandleMirroring; return;
    case 'paint': {
      const ri = num(op.region ?? 0);
      const r = net.regions[ri];
      if (!r) throw new Error(net.regions.length ? `No region ${ri} — this shape has ${net.regions.length}` : 'This shape has no closed area to paint (vector_read → regions)');
      r.fills = op.color === 'none' ? [] : [solid(str(op.color))];
      delete r.fillStyleId;
      return;
    }
    default:
      throw new Error(`Unknown op "${String(op.op)}"`);
  }
}

async function saveNet(node: VectorNode, net: Net) {
  for (const s of net.segments) { if (!s.tangentStart) delete s.tangentStart; if (!s.tangentEnd) delete s.tangentEnd; }
  for (const r of net.regions) if (!r.fills) delete r.fills;
  await node.setVectorNetworkAsync(net as unknown as VectorNetwork);
}

function bend(net: Net, p: P) {
  const amount = num(p.amount);
  const sides = p.side === 'all' || !p.side ? [...SIDES] : [str(p.side)];
  const corners = p.corner === 'all' || !p.corner ? [...CORNERS] : [str(p.corner)];
  switch (p.effect) {
    case 'bulge':
      for (const side of sides) for (const i of pick(net, side, 'segments')) curveSegment(net, i, amount);
      return;
    case 'wave': {
      const count = Math.max(1, Math.round(num(p.count ?? 3)));
      for (const side of sides) {
        for (const first of pick(net, side, 'segments')) {
          const pieces = [first];
          let cur = first;
          for (let k = count * 2; k > 1; k--) { cur = splitSegment(net, cur, 1 / k).segment; pieces.push(cur); }
          pieces.forEach((si, i) => curveSegment(net, si, i % 2 ? -amount : amount));
        }
      }
      return;
    }
    case 'pinch':
    case 'skew':
      for (const side of sides) {
        const along = side === 'top' || side === 'bottom' ? 'x' : 'y';
        const list = pick(net, side, 'points').sort((a, b) => net.vertices[a][along] - net.vertices[b][along]);
        if (p.effect === 'skew') { for (const i of list) net.vertices[i][along] += amount; continue; }
        if (list.length < 2) throw new Error(`The ${side} side has one point — nothing to pinch`);
        net.vertices[list[0]][along] += amount;
        net.vertices[list[list.length - 1]][along] -= amount;
      }
      return;
    case 'chamfer': {
      const list = [...new Set(corners.map((c) => cornerPoint(net, c)))].sort((a, b) => b - a);
      for (const vi of list) chamfer(net, vi, amount);
      return;
    }
    case 'round':
      for (const c of corners) net.vertices[cornerPoint(net, c)].cornerRadius = amount;
      return;
    default:
      throw new Error(`Unknown effect "${String(p.effect)}"`);
  }
}

// ---------- strokes: Figma's Stroke settings (Basic · Dynamic · Brush) ----------

const STRETCH_BRUSHES = ['HEIST', 'BLOCKBUSTER', 'GRINDHOUSE', 'BIOPIC', 'SPAGHETTI_WESTERN', 'SLASHER', 'HARDBOILED', 'VERITE', 'EPIC', 'SCREWBALL', 'ROM_COM', 'NOIR', 'PROPAGANDA', 'MELODRAMA', 'NEW_WAVE'];
const SCATTER_BRUSHES = ['BUBBLEGUM', 'WITCH_HOUSE', 'SHOEGAZE', 'HONKY_TONK', 'SCREAMO', 'DRONE', 'DOO_WOP', 'SPOKEN_WORD', 'VAPORWAVE', 'OI'];

function strokeSummary(node: SceneNode) {
  const g = node as SceneNode & GeometryMixin & Partial<ComplexStrokesMixin> & { strokeJoin?: StrokeJoin | symbol; dashPattern?: readonly number[] };
  const out: Record<string, unknown> = { id: node.id, name: node.name, type: node.type };
  const st = g.strokes?.[0];
  if (st?.type === 'SOLID') out.color = toHex(st.color);
  if (typeof g.strokeWeight === 'number') out.weight = g.strokeWeight;
  if ('strokeAlign' in g) out.align = g.strokeAlign;
  if (g.dashPattern?.length) out.dash = g.dashPattern;
  if (typeof g.strokeCap === 'string') out.cap = g.strokeCap; else if (g.strokeCap) out.cap = 'mixed';
  if (typeof g.strokeJoin === 'string') out.join = g.strokeJoin;
  const vw = g.variableWidthStrokeProperties;
  if (vw && (vw.widthProfile !== 'UNIFORM' || ('variableWidthPoints' in vw && vw.variableWidthPoints.length))) out.width = vw;
  if (g.complexStrokeProperties && g.complexStrokeProperties.type !== 'BASIC') out.stroke = g.complexStrokeProperties;
  return out;
}

async function setStroke(p: P) {
  const node = await getSceneNode(p.nodeId);
  if (!('strokes' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no strokes`);
  if (has(p, 'color')) node.strokes = p.color === 'none' ? [] : [solid(str(p.color))];
  if (has(p, 'weight')) node.strokeWeight = num(p.weight);
  if (has(p, 'align')) node.strokeAlign = p.align as 'INSIDE' | 'OUTSIDE' | 'CENTER';
  const shape = node as SceneNode & GeometryMixin & MinimalStrokesMixin;
  const need = (what: string, ok: boolean) => { if (!ok) throw new Error(`${node.name} (${node.type}) can't take ${what}`); };
  if (has(p, 'dash')) { need('dashes', 'dashPattern' in node); (node as SceneNode & { dashPattern: number[] }).dashPattern = p.dash === 'solid' ? [] : (p.dash as number[]); }
  if (has(p, 'join')) { need('a join', 'strokeJoin' in node); (node as SceneNode & { strokeJoin: StrokeJoin }).strokeJoin = p.join as StrokeJoin; }
  if (has(p, 'miterAngle')) { need('a miter angle', 'strokeMiterLimit' in node); shape.strokeMiterLimit = 1 / Math.sin((num(p.miterAngle) * Math.PI) / 360); }
  if (has(p, 'cap')) { need('end caps', 'strokeCap' in node); shape.strokeCap = p.cap as StrokeCap; }
  if (has(p, 'startCap') || has(p, 'endCap')) {
    if (node.type !== 'VECTOR') throw new Error(`startCap / endCap are for open vector paths; ${node.name} is a ${node.type} — use cap for both ends`);
    const net = netOf(node);
    const ends = net.vertices.map((_, i) => i).filter((i) => net.segments.filter((s) => s.start === i || s.end === i).length === 1);
    if (ends.length < 2) throw new Error(`${node.name} is a closed path — it has no ends`);
    const first = net.segments.find((s) => ends.includes(s.start))?.start ?? ends[0];
    const last = ends.find((i) => i !== first) ?? ends[1];
    if (has(p, 'startCap')) net.vertices[first].strokeCap = p.startCap as StrokeCap;
    if (has(p, 'endCap')) net.vertices[last].strokeCap = p.endCap as StrokeCap;
    await saveNet(node, net);
  }
  const complex = node as SceneNode & ComplexStrokesMixin;
  const kind = p.kind ?? (p.brush ? 'brush' : p.dynamic ? 'dynamic' : undefined);
  if (kind) need('dynamic or brush strokes', 'complexStrokeProperties' in node);
  if (kind === 'basic') complex.complexStrokeProperties = { type: 'BASIC' };
  if (kind === 'dynamic') {
    const was = complex.complexStrokeProperties.type === 'DYNAMIC' ? (complex.complexStrokeProperties as DynamicStrokeProperties) : undefined;
    const d = (p.dynamic ?? {}) as P;
    complex.complexStrokeProperties = {
      type: 'DYNAMIC',
      frequency: has(d, 'frequency') ? num(d.frequency) : was?.frequency ?? 0.75,
      wiggle: has(d, 'wiggle') ? num(d.wiggle) : was?.wiggle ?? 0.3,
      smoothen: has(d, 'smoothen') ? num(d.smoothen) : was?.smoothen ?? 0.5,
    };
  }
  if (kind === 'brush') {
    const b = (p.brush ?? {}) as P;
    const was = complex.complexStrokeProperties.type === 'BRUSH' ? (complex.complexStrokeProperties as BrushStrokeProperties) : undefined;
    const name = has(b, 'name') ? str(b.name).trim().toUpperCase().replace(/[\s-]+/g, '_') : was?.brushName;
    if (!name) throw new Error('brush needs a name');
    if (STRETCH_BRUSHES.includes(name)) {
      await figma.loadBrushesAsync('STRETCH');
      const w = was?.brushType === 'STRETCH' ? was : undefined;
      complex.complexStrokeProperties = { type: 'BRUSH', brushType: 'STRETCH', brushName: name as StretchBrushProperties['brushName'], direction: (b.direction as 'FORWARD' | 'BACKWARD') ?? w?.direction ?? 'FORWARD' };
    } else if (SCATTER_BRUSHES.includes(name)) {
      await figma.loadBrushesAsync('SCATTER');
      const w = was?.brushType === 'SCATTER' ? was : undefined;
      const pickN = (k: string, dflt: number) => (has(b, k) ? num(b[k]) : ((w as unknown as Record<string, number>)?.[k] ?? dflt));
      complex.complexStrokeProperties = {
        type: 'BRUSH', brushType: 'SCATTER', brushName: name as ScatterBrushProperties['brushName'],
        gap: pickN('gap', 1), wiggle: pickN('wiggle', 0), sizeJitter: pickN('sizeJitter', 0), angularJitter: pickN('angularJitter', 0), rotation: pickN('rotation', 0),
      };
    } else throw new Error(`No brush "${String(b.name)}". Stretch: ${STRETCH_BRUSHES.join(', ')}. Scatter: ${SCATTER_BRUSHES.join(', ')}`);
  }
  if (has(p, 'widthProfile') || has(p, 'widthPoints')) {
    need('a width profile', 'variableWidthStrokeProperties' in node);
    complex.variableWidthStrokeProperties = has(p, 'widthPoints')
      ? { widthProfile: 'CUSTOM', variableWidthPoints: p.widthPoints as VariableWidthPoint[] }
      : { widthProfile: p.widthProfile as PresetVariableWidthStrokeProperties['widthProfile'] };
  }
  return strokeSummary(node);
}

// ---------- guards: checked here, in the plugin, so every AI client gets them ----------

// Nodes made through wridge carry this mark; anything without it is the user's work.
const CREATED_KEY = 'wara-created';
function markCreated(node: BaseNode) {
  if ('setPluginData' in node) node.setPluginData(CREATED_KEY, '1');
}
const createdByWara = (node: BaseNode) => 'getPluginData' in node && node.getPluginData(CREATED_KEY) === '1';

// Code is written only from approved designs. Components and Foundations are not screens, so they pass.
function approvalProblem(node: SceneNode): string | null {
  for (let n: BaseNode | null = node; n && n.type !== 'PAGE'; n = n.parent) {
    if (n.type === 'COMPONENT' || n.type === 'COMPONENT_SET') return null;
    if ((n.type === 'FRAME' || n.type === 'SECTION') && FOUNDATIONS_NAME.test(n.name.trim())) return null;
  }
  const screen = screenOf(node);
  if (!screen) return null;
  const status = screenStatus(screen);
  if (status === 'approved') return null;
  return `"${screen.name}" is ${status ? `in a "${status}" section` : 'not in a review section'}, so it is not approved for code. Show the design to the user and get approval (section "<Flow> · Approved"). Pass userApproved: true only if the user said to code it without that review.`;
}

// A section goes to review / approval only with its prototype wired.
// A trailing review status, kept with its separator ("Checkout · Prototype · In review").
const REVIEW_SUFFIX = /(\s+[·\-–]\s+)(in review|review|approved)\s*$/i;
const OVERLAY_NAME = /dialog|toast|sheet|modal|popup|overlay/i;
async function reviewProblem(node: BaseNode, name: string): Promise<string | null> {
  const title = parseSectionTitle(name);
  // Only Prototype sections need a wired prototype; Slides, Storyboard… go to review as they are.
  if (node.type !== 'SECTION' || !title.prototype || (title.status !== 'review' && title.status !== 'approved')) return null;
  const screens = listScreens(node);
  if (screens.length < 2 && !screens.some((s) => OVERLAY_NAME.test(s.name))) return null;
  const ids = new Set(screens.map((s) => s.id));
  const names = new Set(screens.map((s) => s.name));
  graphCache.clear();
  const r = (await handlers.check_flows({ limit: 20 })) as {
    flows: Array<{ nodeId: string }>;
    deadEnds: Array<{ id: string; name: string }>;
    orphans: Array<{ id: string; name: string }>;
    brokenLinks: Array<{ screen: string; from: string }>;
  };
  const issues: string[] = [];
  if (!r.flows.some((f) => ids.has(f.nodeId))) issues.push('no flow starts in this section (set_flow_start)');
  const dead = r.deadEnds.filter((s) => ids.has(s.id)).map((s) => s.name);
  if (dead.length) issues.push(`dead ends: ${dead.join(', ')}`);
  const orphans = r.orphans.filter((s) => ids.has(s.id)).map((s) => s.name);
  if (orphans.length) issues.push(`screens nothing links to: ${orphans.join(', ')}`);
  const broken = r.brokenLinks.filter((l) => names.has(l.screen)).map((l) => `${l.screen} › ${l.from}`);
  if (broken.length) issues.push(`broken links: ${broken.join(', ')}`);
  return issues.length
    ? `Not ready for "${name.trim()}": ${issues.join('; ')}. Wire the prototype (prototyping skill) and run check_flows. Pass allowFlowIssues: true only if the user accepts these as they are.`
    : null;
}

async function guardName(node: BaseNode, name: string, p: P) {
  if (p.allowFlowIssues) return;
  const problem = await reviewProblem(node, name);
  if (problem) throw new Error(problem);
}

function attach(node: SceneNode, parent: BaseNode & ChildrenMixin, p: P) {
  markCreated(node);
  parent.appendChild(node);
  if (has(p, 'x')) node.x = num(p.x);
  if (has(p, 'y')) node.y = num(p.y);
  applySizing(node, p);
}

// ---------- text helpers ----------

async function loadTextFonts(node: TextNode) {
  const fonts =
    node.characters.length > 0 ? node.getRangeAllFontNames(0, node.characters.length) : [node.fontName as FontName];
  await Promise.all(fonts.map((font) => figma.loadFontAsync(font)));
}

function lineHeight(v: unknown): LineHeight {
  if (typeof v === 'number') return { value: v, unit: 'PIXELS' };
  const s = String(v).trim().toLowerCase();
  if (s === 'auto') return { unit: 'AUTO' };
  return { value: parseFloat(s), unit: 'PERCENT' };
}

function letterSpacing(v: unknown): LetterSpacing {
  if (typeof v === 'number') return { value: v, unit: 'PIXELS' };
  return { value: parseFloat(String(v)), unit: 'PERCENT' };
}

async function applyText(node: TextNode, p: P) {
  if (has(p, 'fontFamily') || has(p, 'fontStyle')) {
    const current = node.fontName === figma.mixed ? { family: 'Inter', style: 'Regular' } : node.fontName;
    const font = {
      family: has(p, 'fontFamily') ? str(p.fontFamily) : current.family,
      style: has(p, 'fontStyle') ? str(p.fontStyle) : current.style,
    };
    await figma.loadFontAsync(font);
    node.fontName = font;
  }
  if (has(p, 'fontSize')) node.fontSize = num(p.fontSize);
  if (has(p, 'lineHeight')) node.lineHeight = lineHeight(p.lineHeight);
  if (has(p, 'letterSpacing')) node.letterSpacing = letterSpacing(p.letterSpacing);
  if (has(p, 'textAlign')) node.textAlignHorizontal = p.textAlign as TextNode['textAlignHorizontal'];
  if (has(p, 'fill')) node.fills = [solid(str(p.fill))];
  if (has(p, 'width')) {
    node.textAutoResize = 'HEIGHT';
    node.resize(num(p.width), node.height);
  }
  if (has(p, 'autoResize')) node.textAutoResize = p.autoResize as TextNode['textAutoResize'];
}

// ---------- variables ----------

function formatVariableValue(value: VariableValue): unknown {
  if (typeof value === 'object' && value !== null) {
    if ('type' in value && value.type === 'VARIABLE_ALIAS') return { alias: value.id };
    if ('r' in value) return toHex(value, 'a' in value ? value.a : 1);
  }
  return value;
}

// ---------- shapes ----------

// Boolean ops, flatten and masks need siblings; the result goes where the lowest one was.
async function siblings(ids: unknown) {
  const nodes = await Promise.all((ids as string[]).map((id) => getSceneNode(id)));
  const parent = nodes[0].parent;
  if (!parent) throw new Error(`Node ${nodes[0].id} has no parent`);
  if (nodes.some((n) => n.parent !== parent)) throw new Error('All layers must share the same parent');
  const index = Math.min(...nodes.map((n) => parent.children.indexOf(n)));
  return { nodes, parent, index };
}

// Recolors the drawn shapes only: the wrapping frame/groups keep their (usually empty or white)
// background, otherwise the icon turns into a solid square. Per-paint opacity is kept (duotone icons).
const CONTAINER_TYPES = new Set(['FRAME', 'GROUP', 'SECTION', 'COMPONENT', 'COMPONENT_SET', 'INSTANCE']);
function recolor(root: SceneNode, hex: string) {
  const targets = 'findAll' in root ? root.findAll() : [root];
  const swap = (paints: readonly Paint[]) =>
    paints.map((paint) => {
      if (paint.type !== 'SOLID') return paint;
      const next = solid(hex);
      return { ...next, opacity: (next.opacity ?? 1) * (paint.opacity ?? 1), visible: paint.visible ?? true };
    });
  for (const node of targets) {
    if (CONTAINER_TYPES.has(node.type)) continue;
    if ('fills' in node && Array.isArray(node.fills)) node.fills = swap(node.fills as Paint[]);
    if ('strokes' in node) node.strokes = swap(node.strokes);
  }
}

// ---------- prototype ----------

function buildTrigger(p: P): Trigger {
  const type = str(p.trigger);
  const delay = has(p, 'delay') ? num(p.delay) : 0;
  if (type === 'AFTER_TIMEOUT') return { type, timeout: has(p, 'delay') ? delay : 0.8 };
  if (type === 'MOUSE_ENTER' || type === 'MOUSE_LEAVE') return { type, delay, deprecatedVersion: false };
  if (type === 'MOUSE_UP' || type === 'MOUSE_DOWN') return { type, delay };
  return { type: type as 'ON_CLICK' | 'ON_HOVER' | 'ON_PRESS' | 'ON_DRAG' };
}

function buildTransition(p: P): Transition | null {
  const type = str(p.transition);
  if (type === 'INSTANT') return null;
  const easing = { type: p.easing as 'EASE_IN' | 'EASE_OUT' | 'EASE_IN_AND_OUT' | 'LINEAR' };
  const duration = num(p.duration);
  if (p.action === 'SCROLL_TO') return { type: 'SCROLL_ANIMATE', easing, duration };
  if (type === 'DISSOLVE' || type === 'SMART_ANIMATE') return { type, easing, duration };
  return {
    type: type as DirectionalTransition['type'],
    direction: p.direction as DirectionalTransition['direction'],
    matchLayers: false,
    easing,
    duration,
  };
}

function reactionActions(reaction: Reaction): Action[] {
  return reaction.actions ?? (reaction.action ? [reaction.action] : []);
}

async function describeReactions(node: SceneNode & ReactionMixin) {
  const reactions = [];
  for (const reaction of node.reactions) {
    const actions = [];
    for (const action of reactionActions(reaction)) {
      if (action.type === 'NODE') {
        const destination = action.destinationId ? await figma.getNodeByIdAsync(action.destinationId) : null;
        actions.push({
          action: action.navigation,
          destinationId: action.destinationId,
          destinationName: destination?.name,
          transition: action.transition?.type ?? 'INSTANT',
          duration: action.transition?.duration,
        });
      } else if (action.type === 'URL') {
        actions.push({ action: 'URL', url: action.url });
      } else {
        actions.push({ action: action.type });
      }
    }
    reactions.push({ trigger: reaction.trigger?.type ?? null, actions });
  }
  return { nodeId: node.id, name: node.name, type: node.type, reactions };
}

// ---------- design → code ----------

type Json = Record<string, unknown>;

// Must match the server's token naming: styles/variables become kebab-case CSS names.
function tokenName(name: string, id: string): string {
  const kebab = name
    .normalize('NFKD')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return kebab || `token-${id.replace(/[^a-zA-Z0-9]/g, '')}`;
}

class CodeContext {
  readonly usedTokens: Record<string, unknown> = {};
  readonly components = new Map<string, Json>();
  readonly assets: Json[] = [];
  readonly fonts = new Set<string>();
  readonly stateCache = new Map<string, Json | undefined>();
  private variables = new Map<string, Variable | null>();
  private styles = new Map<string, BaseStyle | null>();
  private names = new Map<string, string | undefined>();

  constructor(readonly expandInstances: boolean) {}

  async nodeName(id: string): Promise<string | undefined> {
    if (!this.names.has(id)) this.names.set(id, (await figma.getNodeByIdAsync(id))?.name);
    return this.names.get(id);
  }

  async variableToken(id: string, value: unknown): Promise<string | undefined> {
    if (!this.variables.has(id)) this.variables.set(id, await figma.variables.getVariableByIdAsync(id));
    const variable = this.variables.get(id);
    if (!variable) return undefined;
    const token = tokenName(variable.name, variable.id);
    this.usedTokens[token] = value;
    return token;
  }

  async styleToken(id: unknown, value: unknown): Promise<string | undefined> {
    if (typeof id !== 'string' || !id) return undefined;
    if (!this.styles.has(id)) this.styles.set(id, await figma.getStyleByIdAsync(id));
    const style = this.styles.get(id);
    if (!style) return undefined;
    const token = tokenName(style.name, style.id);
    this.usedTokens[token] = value;
    return token;
  }
}

const tokenOr = (token: string | undefined, value: unknown) => (token ? { token, value } : value);

async function numericValue(node: SceneNode, field: string, value: number, ctx: CodeContext) {
  const bound = ('boundVariables' in node ? node.boundVariables : undefined) as
    | Record<string, VariableAlias | VariableAlias[] | undefined>
    | undefined;
  const alias = bound?.[field];
  const token = alias && !Array.isArray(alias) ? await ctx.variableToken(alias.id, value) : undefined;
  return tokenOr(token, round(value));
}

async function paintValue(paint: Paint, ctx: CodeContext): Promise<unknown> {
  if (paint.type === 'SOLID') {
    const hex = toHex(paint.color, paint.opacity ?? 1);
    const alias = paint.boundVariables?.color;
    return tokenOr(alias ? await ctx.variableToken(alias.id, hex) : undefined, hex);
  }
  if (paint.type === 'IMAGE') return { image: true, scaleMode: paint.scaleMode };
  if (paint.type.startsWith('GRADIENT_')) {
    const gradient = paint as GradientPaint;
    return {
      gradient: gradient.type.replace('GRADIENT_', '').toLowerCase(),
      stops: gradient.gradientStops.map((s) => ({ color: toHex(s.color, s.color.a), position: round(s.position) })),
    };
  }
  return null;
}

async function paintsValue(paints: unknown, styleId: unknown, ctx: CodeContext) {
  if (!Array.isArray(paints)) return undefined;
  const visible = (paints as Paint[]).filter((p) => p.visible !== false);
  if (!visible.length) return undefined;
  const values = await Promise.all(visible.map((p) => paintValue(p, ctx)));
  const single = values.length === 1 ? values[0] : values;
  const styleToken = await ctx.styleToken(styleId, single);
  return tokenOr(styleToken, single);
}

const VECTOR_TYPES = new Set<string>(['VECTOR', 'BOOLEAN_OPERATION', 'STAR', 'POLYGON']);
const ICON_PARTS = new Set<string>(['VECTOR', 'BOOLEAN_OPERATION', 'STAR', 'POLYGON', 'GROUP', 'ELLIPSE', 'RECTANGLE', 'LINE']);

function isIcon(node: SceneNode): boolean {
  if (VECTOR_TYPES.has(node.type)) return true;
  if ((node.type === 'FRAME' || node.type === 'GROUP') && node.width <= 64 && node.height <= 64) {
    const parts = node.findAll();
    return parts.some((n) => VECTOR_TYPES.has(n.type)) && parts.every((n) => ICON_PARTS.has(n.type));
  }
  return false;
}

function hasImageFill(node: SceneNode): boolean {
  return 'fills' in node && Array.isArray(node.fills) && (node.fills as Paint[]).some((p) => p.type === 'IMAGE' && p.visible !== false);
}

const ALIGN: Record<string, string> = { MIN: 'start', CENTER: 'center', MAX: 'end', SPACE_BETWEEN: 'between', BASELINE: 'baseline' };

function sizeOf(node: SceneNode, isRoot: boolean): Json {
  const out: Json = {};
  const axis = (sizing: string, px: number) => (sizing === 'FILL' ? 'fill' : sizing === 'HUG' ? 'hug' : round(px));
  if (!isRoot && 'layoutSizingHorizontal' in node) {
    out.width = axis(node.layoutSizingHorizontal, node.width);
    out.height = axis(node.layoutSizingVertical, node.height);
  } else {
    out.width = round(node.width);
    out.height = round(node.height);
  }
  if ('minWidth' in node) {
    if (node.minWidth !== null) out.minWidth = node.minWidth;
    if (node.maxWidth !== null) out.maxWidth = node.maxWidth;
    if (node.minHeight !== null) out.minHeight = node.minHeight;
    if (node.maxHeight !== null) out.maxHeight = node.maxHeight;
  }
  return out;
}

function lineHeightValue(lh: LineHeight) {
  if (lh.unit === 'AUTO') return 'normal';
  return lh.unit === 'PIXELS' ? `${round(lh.value)}px` : `${round(lh.value)}%`;
}

function letterSpacingValue(ls: LetterSpacing) {
  return ls.unit === 'PIXELS' ? `${round(ls.value)}px` : `${round(ls.value / 100)}em`;
}

async function textValue(node: TextNode, ctx: CodeContext): Promise<Json> {
  const segments = node.getStyledTextSegments(['fontName', 'fontWeight', 'fontSize', 'fills', 'textDecoration', 'lineHeight', 'letterSpacing']);
  const describe = async (s: (typeof segments)[number]) => {
    ctx.fonts.add(s.fontName.family);
    return {
      family: s.fontName.family,
      weight: s.fontWeight,
      italic: /italic|oblique/i.test(s.fontName.style) || undefined,
      size: round(s.fontSize),
      lineHeight: lineHeightValue(s.lineHeight),
      letterSpacing: s.letterSpacing.value === 0 ? undefined : letterSpacingValue(s.letterSpacing),
      color: await paintsValue(s.fills, undefined, ctx),
      decoration: s.textDecoration !== 'NONE' ? s.textDecoration.toLowerCase() : undefined,
    };
  };
  const base = await describe(segments[0]);
  const out: Json = {
    content: node.characters,
    ...base,
    align: node.textAlignHorizontal === 'LEFT' ? undefined : node.textAlignHorizontal.toLowerCase(),
    transform: node.textCase !== figma.mixed && node.textCase !== 'ORIGINAL' ? node.textCase.toLowerCase() : undefined,
    truncate: node.textAutoResize === 'TRUNCATE' || undefined,
  };
  const textStyle = await ctx.styleToken(node.textStyleId, { family: base.family, size: base.size, weight: base.weight });
  if (textStyle) out.textStyle = textStyle;
  if (segments.length > 1) {
    out.spans = await Promise.all(segments.map(async (s) => ({ text: s.characters, ...(await describe(s)) })));
  }
  return out;
}

// Figma clips every frame by default; only report it when it changes what you see.
function clippingMatters(node: SceneNode): boolean {
  if ('cornerRadius' in node && node.cornerRadius !== 0) return true;
  if (!('children' in node)) return false;
  return node.children.some(
    (c) => c.visible && (c.x < -0.5 || c.y < -0.5 || c.x + c.width > node.width + 0.5 || c.y + c.height > node.height + 0.5),
  );
}

const isZero = (v: unknown) => v === 0;

// CSS-style shorthand: 16 | [y, x] | [top, right, bottom, left]; undefined when all zero.
function paddingShorthand([t, r, b, l]: unknown[]): unknown {
  if ([t, r, b, l].every(isZero)) return undefined;
  const same = (a: unknown, c: unknown) => JSON.stringify(a) === JSON.stringify(c);
  if (same(t, b) && same(r, l)) return same(t, r) ? t : [t, r];
  return [t, r, b, l];
}

async function styleValue(node: SceneNode, ctx: CodeContext): Promise<Json> {
  const style: Json = {};
  if ('fills' in node && node.type !== 'TEXT') {
    const background = await paintsValue(node.fills, node.fillStyleId, ctx);
    if (background) style.background = background;
  }
  if ('strokes' in node && node.strokes.length) {
    const color = await paintsValue(node.strokes, 'strokeStyleId' in node ? node.strokeStyleId : undefined, ctx);
    if (color) {
      const weight =
        node.strokeWeight === figma.mixed && 'strokeTopWeight' in node
          ? [node.strokeTopWeight, node.strokeRightWeight, node.strokeBottomWeight, node.strokeLeftWeight]
          : node.strokeWeight;
      style.border = { color, width: weight, align: node.strokeAlign.toLowerCase() };
    }
  }
  if (node.type === 'ELLIPSE') {
    style.radius = 'full';
  } else if ('cornerRadius' in node) {
    if (node.cornerRadius === figma.mixed && 'topLeftRadius' in node) {
      style.radius = [node.topLeftRadius, node.topRightRadius, node.bottomRightRadius, node.bottomLeftRadius];
    } else if (typeof node.cornerRadius === 'number' && node.cornerRadius > 0) {
      style.radius = await numericValue(node, 'cornerRadius', node.cornerRadius, ctx);
    }
  }
  if ('effects' in node && node.effects.length) {
    const shadows = node.effects
      .filter((e): e is DropShadowEffect | InnerShadowEffect => (e.type === 'DROP_SHADOW' || e.type === 'INNER_SHADOW') && e.visible)
      .map((e) => ({
        x: e.offset.x,
        y: e.offset.y,
        blur: e.radius,
        spread: e.spread ?? 0,
        color: toHex(e.color, e.color.a),
        inset: e.type === 'INNER_SHADOW' || undefined,
      }));
    const effectStyle = 'effectStyleId' in node ? await ctx.styleToken(node.effectStyleId, shadows) : undefined;
    if (shadows.length) style.shadow = tokenOr(effectStyle, shadows);
    const blur = node.effects.find((e) => (e.type === 'LAYER_BLUR' || e.type === 'BACKGROUND_BLUR') && e.visible);
    if (blur && 'radius' in blur) style[blur.type === 'LAYER_BLUR' ? 'blur' : 'backdropBlur'] = blur.radius;
  }
  if ('opacity' in node && node.opacity < 1) style.opacity = round(node.opacity);
  if ('clipsContent' in node && node.clipsContent && clippingMatters(node)) style.overflow = 'hidden';
  if ('blendMode' in node && node.blendMode !== 'NORMAL' && node.blendMode !== 'PASS_THROUGH') {
    style.blendMode = node.blendMode.toLowerCase();
  }
  return style;
}

function assetFor(node: SceneNode, format: 'svg' | 'png', ctx: CodeContext) {
  ctx.assets.push({ nodeId: node.id, name: node.name, format });
  return format;
}

const PIN_X: Record<ConstraintType, string> = { MIN: 'left', MAX: 'right', CENTER: 'center', STRETCH: 'left-right', SCALE: 'scale' };
const PIN_Y: Record<ConstraintType, string> = { MIN: 'top', MAX: 'bottom', CENTER: 'center', STRETCH: 'top-bottom', SCALE: 'scale' };

// Only reported when it differs from the default top-left pin.
function pinOf(node: SceneNode): Json | undefined {
  if (!('constraints' in node)) return undefined;
  const { horizontal, vertical } = node.constraints;
  if (horizontal === 'MIN' && vertical === 'MIN') return undefined;
  return { x: PIN_X[horizontal], y: PIN_Y[vertical] };
}

// The click/press interaction, so code can turn it into a route or link.
async function linkOf(node: SceneNode, ctx: CodeContext): Promise<Json | undefined> {
  if (!('reactions' in node)) return undefined;
  const reaction = node.reactions.find((r) => r.trigger?.type === 'ON_CLICK' || r.trigger?.type === 'ON_PRESS');
  const action = reaction ? reactionActions(reaction)[0] : undefined;
  if (!action) return undefined;
  if (action.type === 'BACK') return { action: 'back' };
  if (action.type === 'CLOSE') return { action: 'close' };
  if (action.type === 'URL') return { action: 'url', url: action.url };
  if (action.type !== 'NODE' || !action.destinationId) return undefined;
  return { action: action.navigation.toLowerCase(), to: await ctx.nodeName(action.destinationId), toId: action.destinationId };
}

function trackValue(track: GridTrackSize): unknown {
  if (track.type === 'FLEX') return `${track.value ?? 1}fr`;
  if (track.type === 'HUG') return 'auto';
  return round(track.value ?? 0);
}

const GRID_ALIGN: Record<string, string | undefined> = { MIN: 'start', CENTER: 'center', MAX: 'end', AUTO: undefined };

function gridPlacement(node: SceneNode): Json | undefined {
  if (!('gridRowAnchorIndex' in node)) return undefined;
  const out: Json = {
    row: node.gridRowAnchorIndex + 1,
    column: node.gridColumnAnchorIndex + 1,
    rowSpan: node.gridRowSpan > 1 ? node.gridRowSpan : undefined,
    columnSpan: node.gridColumnSpan > 1 ? node.gridColumnSpan : undefined,
    justify: GRID_ALIGN[node.gridChildHorizontalAlign],
    align: GRID_ALIGN[node.gridChildVerticalAlign],
  };
  return out;
}

// Other variants of the instance's set that differ in exactly one property (e.g. State=Hover),
// reported as what changes compared with the current variant.
async function variantStates(main: ComponentNode, set: ComponentSetNode, ctx: CodeContext): Promise<Json | undefined> {
  if (ctx.stateCache.has(main.id)) return ctx.stateCache.get(main.id);
  const current = main.variantProperties ?? {};
  const scratch = new CodeContext(false);
  const base = flatten(await codeNode(main, 3, scratch, true));
  const states: Json = {};
  for (const variant of set.children) {
    if (variant.type !== 'COMPONENT' || variant.id === main.id) continue;
    const props = variant.variantProperties ?? {};
    const changed = Object.keys(props).filter((k) => props[k] !== current[k]);
    if (changed.length !== 1) continue;
    const other = flatten(await codeNode(variant, 3, scratch, true));
    const diff = Object.fromEntries(
      Object.entries(other).filter(([path, v]) => path !== 'name' && JSON.stringify(v) !== JSON.stringify(base[path])),
    );
    if (Object.keys(diff).length) states[`${changed[0]}=${props[changed[0]]}`] = diff;
    if (Object.keys(states).length >= 8) break;
  }
  Object.assign(ctx.usedTokens, scratch.usedTokens);
  const result = Object.keys(states).length ? states : undefined;
  ctx.stateCache.set(main.id, result);
  return result;
}

async function codeNode(node: SceneNode, depth: number, ctx: CodeContext, isRoot = false): Promise<Json | null> {
  if (!node.visible) return null;
  const out: Json = { id: node.id, name: node.name };
  const parent = node.parent;
  const inAutoLayout = !isRoot && parent !== null && 'layoutMode' in parent && parent.layoutMode !== 'NONE';
  const absolute = !isRoot && (!inAutoLayout || ('layoutPositioning' in node && node.layoutPositioning === 'ABSOLUTE'));
  const inGrid = inAutoLayout && parent !== null && 'layoutMode' in parent && parent.layoutMode === 'GRID';
  const place = () => {
    Object.assign(out, sizeOf(node, isRoot));
    if (absolute) {
      out.position = { x: round(node.x), y: round(node.y) };
      const pin = pinOf(node);
      if (pin) out.pin = pin;
    }
    if (inGrid) out.grid = gridPlacement(node);
  };
  const link = await linkOf(node, ctx);
  if (link) out.link = link;

  if (node.type === 'INSTANCE') {
    const main = await node.getMainComponentAsync();
    const set = main?.parent?.type === 'COMPONENT_SET' ? main.parent : null;
    const component = {
      name: set?.name ?? main?.name ?? node.name,
      variant: set ? main?.name : undefined,
      props: Object.fromEntries(Object.entries(node.componentProperties).map(([k, v]) => [k.replace(/#[\d:]+$/, ''), v.value])),
    };
    out.kind = 'instance';
    out.component = component;
    // Lottie/<name> components (made by /motion) stand for an animation file: code plays it, never redraws the GIF.
    const lottie = /^lottie\//i.test(component.name) ? /lottie:\s*(\S+)/i.exec((set ?? main)?.description ?? '')?.[1] : undefined;
    if (lottie) { out.kind = 'lottie'; out.lottie = lottie; place(); return out; }
    if (main && set) {
      const states = await variantStates(main, set, ctx);
      if (states) out.states = states;
    }
    ctx.components.set(component.name, { name: component.name, key: set?.key ?? main?.key });
    place();
    if (!ctx.expandInstances) return out;
  } else if (isIcon(node)) {
    out.kind = 'icon';
    out.asset = assetFor(node, 'svg', ctx);
    place();
    const color = 'findAll' in node ? undefined : await paintsValue('fills' in node ? node.fills : undefined, undefined, ctx);
    if (color) out.color = color;
    return out;
  } else if (node.type === 'TEXT') {
    out.kind = 'text';
    place();
    out.text = await textValue(node, ctx);
    return out;
  } else if (node.type === 'LINE') {
    out.kind = 'divider';
    place();
    out.style = await styleValue(node, ctx);
    return out;
  } else {
    out.kind = 'children' in node ? 'frame' : 'shape';
    place();
  }

  if (hasImageFill(node)) out.asset = assetFor(node, 'png', ctx);

  if ('layoutMode' in node && node.layoutMode === 'GRID') {
    out.layout = {
      display: 'grid',
      columns: node.gridColumnSizes.map(trackValue),
      rows: node.gridRowSizes.map(trackValue),
      columnGap: isZero(node.gridColumnGap) ? undefined : await numericValue(node, 'gridColumnGap', node.gridColumnGap, ctx),
      rowGap: isZero(node.gridRowGap) ? undefined : await numericValue(node, 'gridRowGap', node.gridRowGap, ctx),
      padding: paddingShorthand(
        await Promise.all([
          numericValue(node, 'paddingTop', node.paddingTop, ctx),
          numericValue(node, 'paddingRight', node.paddingRight, ctx),
          numericValue(node, 'paddingBottom', node.paddingBottom, ctx),
          numericValue(node, 'paddingLeft', node.paddingLeft, ctx),
        ]),
      ),
    };
  } else if ('layoutMode' in node && node.layoutMode !== 'NONE') {
    // Defaults (gap 0, padding 0, start alignment) are omitted to keep the context short.
    const gap = node.primaryAxisAlignItems === 'SPACE_BETWEEN' ? 0 : await numericValue(node, 'itemSpacing', node.itemSpacing, ctx);
    const justify = ALIGN[node.primaryAxisAlignItems];
    const align = ALIGN[node.counterAxisAlignItems];
    out.layout = {
      direction: node.layoutMode === 'HORIZONTAL' ? 'row' : 'column',
      gap: isZero(gap) ? undefined : gap,
      wrap: node.layoutWrap === 'WRAP' || undefined,
      rowGap: node.layoutWrap === 'WRAP' ? await numericValue(node, 'counterAxisSpacing', node.counterAxisSpacing ?? 0, ctx) : undefined,
      padding: paddingShorthand(
        await Promise.all([
          numericValue(node, 'paddingTop', node.paddingTop, ctx),
          numericValue(node, 'paddingRight', node.paddingRight, ctx),
          numericValue(node, 'paddingBottom', node.paddingBottom, ctx),
          numericValue(node, 'paddingLeft', node.paddingLeft, ctx),
        ]),
      ),
      justify: justify === 'start' ? undefined : justify,
      align: align === 'start' ? undefined : align,
    };
  } else if ('children' in node && node.children.length && node.type !== 'BOOLEAN_OPERATION') {
    out.layout = 'absolute';
  }

  const style = await styleValue(node, ctx);
  if (Object.keys(style).length) out.style = style;

  if ('children' in node && node.type !== 'BOOLEAN_OPERATION') {
    if (depth > 0) {
      const children = [];
      for (const child of node.children) {
        const c = await codeNode(child, depth - 1, ctx);
        if (c) children.push(c);
      }
      if (children.length) out.children = collapseRepeats(children);
    } else if (node.children.length) {
      out.truncatedChildren = node.children.length;
    }
  }
  return out;
}

// ---------- repeated structures ----------

// Keys whose values may differ between items of the same repeated structure.
const VARYING_KEYS = new Set(['content', 'spans', 'color', 'background', 'props', 'variant']);
const MIN_REPEAT = 3;

// Drops ids, names and varying values (and containers left empty), so e.g. a highlighted
// row still matches its plain siblings; the difference shows up in that item's diff.
function structureOf(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(structureOf);
  if (value && typeof value === 'object') {
    const entries = Object.entries(value)
      .filter(([k]) => k !== 'id' && k !== 'name' && !VARYING_KEYS.has(k))
      .map(([k, v]) => [k, structureOf(v)] as const)
      .filter(([, v]) => !(v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === 0));
    return Object.fromEntries(entries);
  }
  return value;
}

function flatten(value: unknown, prefix = '', out: Record<string, unknown> = {}): Record<string, unknown> {
  if (value && typeof value === 'object' && !Array.isArray(value) && !('token' in value)) {
    for (const [k, v] of Object.entries(value)) if (k !== 'id') flatten(v, prefix ? `${prefix}.${k}` : k, out);
  } else if (Array.isArray(value) && value.every((v) => v && typeof v === 'object')) {
    value.forEach((v, i) => flatten(v, `${prefix}.${i}`, out));
  } else {
    out[prefix] = value;
  }
  return out;
}

// Runs of ≥3 adjacent siblings with the same structure become one `repeat` entry:
// the first is kept in full as `template`, the others only list what differs from it.
function collapseRepeats(children: Json[]): Json[] {
  const out: Json[] = [];
  let i = 0;
  while (i < children.length) {
    const signature = JSON.stringify(structureOf(children[i]));
    let j = i + 1;
    while (j < children.length && JSON.stringify(structureOf(children[j])) === signature) j++;
    if (j - i < MIN_REPEAT) {
      out.push(...children.slice(i, j));
    } else {
      const template = children[i];
      const base = flatten(template);
      out.push({
        kind: 'repeat',
        count: j - i,
        note: 'Same structure repeated — implement once (component or map over data); items list only what differs.',
        template,
        items: children.slice(i, j).map((item) => {
          const diff = Object.fromEntries(
            Object.entries(flatten(item)).filter(([path, v]) => JSON.stringify(v) !== JSON.stringify(base[path])),
          );
          return Object.keys(diff).length ? { id: item.id, diff } : { id: item.id };
        }),
      });
    }
    i = j;
  }
  return out;
}

function variableValue(value: VariableValue, names: Map<string, string>): unknown {
  if (typeof value === 'object' && value !== null) {
    if ('type' in value && value.type === 'VARIABLE_ALIAS') return { alias: names.get(value.id) ?? value.id };
    if ('r' in value) return toHex(value, 'a' in value ? value.a : 1);
  }
  return value;
}

// ---------- Foundations page (works on the free plan: no variables/styles needed) ----------
// A page named "Foundations" holds plain layers whose names are token names:
//   Colors/Light, Colors/Dark (or just Colors) → swatches' fill per mode
//   Spacing → swatch width · Radius → corner radius · Typography → text layers · Shadows → drop shadows

type TokenData = {
  collections: Json[];
  variables: Json[];
  paintStyles: Json[];
  textStyles: Json[];
  effectStyles: Json[];
};

const GENERIC_NAME = /^(rectangle|ellipse|frame|vector|group|polygon|star|line|swatch)(\s*\d+)?$/i;
const SECTIONS = {
  colors: /^colou?rs?(?:\s*\/\s*(.+))?$/i,
  spacing: /^spac(?:ing|e)$/i,
  radius: /^(radius|radii|corners?)$/i,
  typography: /^(typography|type|text styles?)$/i,
  shadows: /^(shadows?|elevation|effects?)$/i,
};

// Swatches are often "Rectangle 3" inside a frame named "primary"; use the meaningful name.
function swatchName(node: SceneNode): string {
  const own = node.name.trim();
  const parent = node.parent;
  if (GENERIC_NAME.test(own) && parent && parent.type !== 'PAGE' && parent.type !== 'DOCUMENT') return parent.name.trim();
  return own;
}

// Layers whose name starts with "_" are captions/annotations on the Foundations frame, never tokens.
const isAnnotation = (n: BaseNode) => n.name.trim().startsWith('_');

function leafShapes(root: BaseNode & ChildrenMixin): SceneNode[] {
  return root.findAll((n) => n.visible && !isAnnotation(n) && n.type !== 'TEXT' && !('children' in n && n.children.length > 0));
}

const stripPrefix = (token: string, re: RegExp) => token.replace(re, '') || token;

// "Foundations", also with a suffix added out of habit ("Foundations · Draft").
const FOUNDATIONS_NAME = /^foundations?(\s+[·\-–]\s+.*)?$/i;

// A page named "Foundations", or — so free-plan files needn't spend one of their 3 pages —
// a top-level section/frame named "Foundations" on any page.
async function findFoundations(): Promise<(BaseNode & ChildrenMixin) | null> {
  const page = figma.root.children.find((p) => FOUNDATIONS_NAME.test(p.name.trim()));
  if (page) {
    await page.loadAsync();
    return page;
  }
  for (const p of figma.root.children) {
    await p.loadAsync();
    const holder = p.children.find((n) => (n.type === 'SECTION' || n.type === 'FRAME') && FOUNDATIONS_NAME.test(n.name.trim()));
    if (holder && 'findAll' in holder) return holder;
  }
  return null;
}

async function readFoundations(): Promise<(TokenData & { page: string }) | null> {
  const page = await findFoundations();
  if (!page) return null;
  const ctx = new CodeContext(false);
  const data: TokenData = { collections: [], variables: [], paintStyles: [], textStyles: [], effectStyles: [] };
  const colors = new Map<string, Json & { values: Json }>();
  const modes: string[] = [];
  const scale: Json[] = [];
  const containers = page.findAll((n) => 'children' in n && n.children.length > 0) as Array<SceneNode & ChildrenMixin>;

  for (const section of containers) {
    const title = section.name.trim();
    const colorMatch = SECTIONS.colors.exec(title);
    if (colorMatch) {
      const mode = (colorMatch[1] ?? 'Default').trim();
      if (!modes.includes(mode)) modes.push(mode);
      for (const shape of leafShapes(section)) {
        if (!('fills' in shape) || !Array.isArray(shape.fills)) continue;
        const paint = (shape.fills as Paint[]).find((p) => p.visible !== false);
        if (!paint) continue;
        const label = swatchName(shape);
        const token = `color-${stripPrefix(tokenName(label, shape.id), /^colou?rs?-/)}`;
        if (paint.type === 'SOLID') {
          const entry = colors.get(token) ?? { token, name: label, collection: 'Foundations', type: 'COLOR', values: {} };
          (entry.values as Json)[mode] = toHex(paint.color, paint.opacity ?? 1);
          colors.set(token, entry);
        } else if (paint.type.startsWith('GRADIENT_')) {
          data.paintStyles.push({ token: tokenName(label, shape.id), name: label, value: await paintValue(paint, ctx) });
        }
      }
    } else if (SECTIONS.spacing.test(title)) {
      for (const shape of leafShapes(section)) {
        const label = swatchName(shape);
        scale.push({ token: `spacing-${stripPrefix(tokenName(label, shape.id), /^spac(ing|e)-/)}`, name: label, collection: 'Foundations Scale', type: 'FLOAT', values: { Value: round(shape.width) } });
      }
    } else if (SECTIONS.radius.test(title)) {
      for (const shape of leafShapes(section)) {
        if (!('cornerRadius' in shape) || typeof shape.cornerRadius !== 'number') continue;
        const label = swatchName(shape);
        scale.push({ token: `radius-${stripPrefix(tokenName(label, shape.id), /^(radius|radii|corner)-/)}`, name: label, collection: 'Foundations Scale', type: 'FLOAT', values: { Value: shape.cornerRadius } });
      }
    } else if (SECTIONS.typography.test(title)) {
      for (const text of section.findAll((n) => n.type === 'TEXT' && !isAnnotation(n)) as TextNode[]) {
        if (text.fontName === figma.mixed || text.fontSize === figma.mixed || text.lineHeight === figma.mixed || text.letterSpacing === figma.mixed) continue;
        data.textStyles.push({
          token: `text-${stripPrefix(tokenName(text.name, text.id), /^(text|type)-/)}`,
          name: text.name,
          family: text.fontName.family,
          style: text.fontName.style,
          size: text.fontSize,
          lineHeight: lineHeightValue(text.lineHeight),
          letterSpacing: letterSpacingValue(text.letterSpacing),
        });
      }
    } else if (SECTIONS.shadows.test(title)) {
      for (const shape of leafShapes(section)) {
        if (!('effects' in shape)) continue;
        const shadows = shape.effects
          .filter((e): e is DropShadowEffect | InnerShadowEffect => (e.type === 'DROP_SHADOW' || e.type === 'INNER_SHADOW') && e.visible)
          .map((e) => ({ x: e.offset.x, y: e.offset.y, blur: e.radius, spread: e.spread ?? 0, color: toHex(e.color, e.color.a), inset: e.type === 'INNER_SHADOW' }));
        if (shadows.length) {
          const label = swatchName(shape);
          data.effectStyles.push({ token: `shadow-${stripPrefix(tokenName(label, shape.id), /^(shadow|elevation)-/)}`, name: label, shadows });
        }
      }
    }
  }

  if (colors.size) {
    data.collections.push({ name: 'Foundations', defaultMode: modes.includes('Light') ? 'Light' : modes[0], modes });
    data.variables.push(...colors.values());
  }
  if (scale.length) {
    data.collections.push({ name: 'Foundations Scale', defaultMode: 'Value', modes: ['Value'] });
    data.variables.push(...scale);
  }
  return { page: page.name, ...data };
}

// Values actually used in a design, most frequent first — the raw material for naming tokens
// when a file has no Foundations, variables or styles yet.
function scanValues(root: SceneNode, limit: number) {
  const counts = new Map<string, Map<string, number>>();
  const bump = (kind: string, key: string) => {
    if (!counts.has(kind)) counts.set(kind, new Map());
    const m = counts.get(kind) as Map<string, number>;
    m.set(key, (m.get(key) ?? 0) + 1);
  };
  const nodes = 'findAll' in root ? [root, ...root.findAll((n) => n.visible)] : [root];
  for (const n of nodes) {
    if ('fills' in n && Array.isArray(n.fills)) {
      for (const p of n.fills as Paint[]) {
        if (p.type === 'SOLID' && p.visible !== false) bump(n.type === 'TEXT' ? 'textColors' : 'backgroundColors', toHex(p.color, p.opacity ?? 1));
      }
    }
    if ('strokes' in n) for (const p of n.strokes) if (p.type === 'SOLID') bump('borderColors', toHex(p.color, p.opacity ?? 1));
    if (n.type === 'TEXT' && n.fontName !== figma.mixed && n.fontSize !== figma.mixed && n.lineHeight !== figma.mixed) {
      bump('typography', `${n.fontName.family} ${n.fontName.style} ${n.fontSize}/${lineHeightValue(n.lineHeight)}`);
    }
    if ('cornerRadius' in n && typeof n.cornerRadius === 'number' && n.cornerRadius > 0) bump('radii', String(n.cornerRadius));
    if ('layoutMode' in n && n.layoutMode !== 'NONE') {
      if (n.itemSpacing > 0) bump('gaps', String(n.itemSpacing));
      for (const v of [n.paddingTop, n.paddingRight, n.paddingBottom, n.paddingLeft]) if (v > 0) bump('paddings', String(v));
    }
    if ('effects' in n) {
      for (const e of n.effects) {
        if (e.type === 'DROP_SHADOW' && e.visible) bump('shadows', `${e.offset.x} ${e.offset.y} ${e.radius} ${e.spread ?? 0} ${toHex(e.color, e.color.a)}`);
      }
    }
  }
  return Object.fromEntries(
    [...counts].map(([kind, m]) => [kind, [...m].sort((a, b) => b[1] - a[1]).slice(0, limit).map(([value, count]) => ({ value, count }))]),
  );
}

// ---------- design graph: screens, flows, component usage ----------
// Built per page and cached in memory; any document change on a page drops that page's cache,
// so answers never come from stale data.

type ComponentUse = { count: number; variants: Record<string, number>; instanceIds: string[] };
type ScreenLink = { fromId: string; from: string; trigger: string; action: string; toId?: string; toScreenId?: string; toScreen?: string; broken?: boolean };
type ScreenInfo = {
  id: string;
  name: string;
  pageId: string;
  page: string;
  width: number;
  height: number;
  components: Record<string, ComponentUse>;
  links: ScreenLink[];
  status?: string;
};
type PageGraph = { id: string; name: string; flows: Array<{ nodeId: string; name: string }>; screens: ScreenInfo[] };

const graphCache = new Map<string, PageGraph>();
let watchingChanges = false;

function pageOf(node: BaseNode): PageNode | null {
  let current: BaseNode | null = node;
  while (current && current.type !== 'PAGE') current = current.parent;
  return current;
}

function watchChanges() {
  if (watchingChanges) return;
  watchingChanges = true;
  figma.on('documentchange', (event) => {
    for (const change of event.documentChanges) {
      const node = 'node' in change ? change.node : null;
      const page = node && !('removed' in node && node.removed) ? pageOf(node as BaseNode) : null;
      graphCache.delete((page ?? figma.currentPage).id);
    }
  });
}

// Screens = frames placed directly on a page or inside (nested) sections.
// Only Prototype work is in the design graph: sections of other kinds (Slides, Storyboard…) and
// Foundations are skipped. Loose frames and old-style section names count as Prototype.
function listScreens(container: BaseNode & ChildrenMixin): FrameNode[] {
  const screens: FrameNode[] = [];
  for (const child of container.children) {
    if (FOUNDATIONS_NAME.test(child.name.trim())) continue;
    if (child.type === 'FRAME') screens.push(child);
    else if (child.type === 'SECTION' && parseSectionTitle(child.name).prototype) screens.push(...listScreens(child));
  }
  return screens;
}

// Sections left out of the design graph, so design_map can say what it skipped.
function otherSections(container: BaseNode & ChildrenMixin, out: Array<{ name: string; kind: string }> = []) {
  for (const child of container.children) {
    if (child.type !== 'SECTION') continue;
    const t = parseSectionTitle(child.name);
    if (!t.prototype) out.push({ name: child.name, kind: t.kind as string });
    else otherSections(child, out);
  }
  return out;
}

// <section-title> — server/test reads this block from the compiled code.js and tests it.
// Section names: "<Name> · <Kind> · <Status>" (· or - between parts). Status last: Draft / In review /
// Approved. Kind: Prototype / App (screens of an app), or anything else (Slides, Storyboard, Graphic…).
// "<Name> · <Status>" and loose frames are the older form and count as Prototype.
const SECTION_STATUS = /^(draft|in review|review|approved)$/i;
const SECTION_KINDS = /^(prototype|app|web|mobile|slides?|deck|storyboard|video|graphics?|poster|social|print|diagram|illustration|doc|docs|other)$/i;
function parseSectionTitle(name: string) {
  const parts = String(name).trim().split(/\s+[·\-–]\s+/);
  let status: string | null = null;
  if (parts.length > 1 && SECTION_STATUS.test(parts[parts.length - 1])) {
    const word = (parts.pop() as string).toLowerCase();
    status = word === 'approved' ? 'approved' : word === 'draft' ? 'draft' : 'review';
  }
  let kind: string | null = null;
  // A kind sits right before the status ("Pitch · Slides · Draft"), or ends a name with no status ("Pitch · Slides").
  if (parts.length > 1 && (status || SECTION_KINDS.test(parts[parts.length - 1]))) kind = (parts.pop() as string).toLowerCase();
  const prototype = kind === null || /^(prototype|app|web|mobile)$/.test(kind);
  return { title: parts.join(' · '), kind, status, prototype };
}
// </section-title>

// Review status / kind of a screen come from the nearest section that states them.
function screenStatus(screen: BaseNode): string | undefined {
  for (let p = screen.parent; p && p.type !== 'PAGE'; p = p.parent) {
    if (p.type !== 'SECTION') continue;
    const { status } = parseSectionTitle(p.name);
    if (status) return status;
  }
  return undefined;
}

function screenOf(node: BaseNode): SceneNode | null {
  let current: BaseNode | null = node;
  while (current && current.parent && current.parent.type !== 'PAGE' && current.parent.type !== 'SECTION') current = current.parent;
  return current && current.type !== 'PAGE' && current.type !== 'DOCUMENT' ? (current as SceneNode) : null;
}

function insideInstance(node: BaseNode, stop: BaseNode): boolean {
  for (let p = node.parent; p && p !== stop; p = p.parent) if (p.type === 'INSTANCE') return true;
  return false;
}

async function componentFamily(instance: InstanceNode) {
  const main = await instance.getMainComponentAsync();
  if (!main) return null;
  const set = main.parent?.type === 'COMPONENT_SET' ? main.parent : null;
  return { family: set?.name ?? main.name, variant: set ? main.name : undefined };
}

async function buildPageGraph(page: PageNode): Promise<PageGraph> {
  const cached = graphCache.get(page.id);
  if (cached) return cached;
  await page.loadAsync();
  const screens: ScreenInfo[] = [];
  const nameCache = new Map<string, { id?: string; name?: string } | null>();
  for (const screen of listScreens(page)) {
    const components: Record<string, ComponentUse> = {};
    for (const instance of screen.findAllWithCriteria({ types: ['INSTANCE'] })) {
      if (insideInstance(instance, screen)) continue;
      const family = await componentFamily(instance);
      if (!family) continue;
      const use = (components[family.family] ??= { count: 0, variants: {}, instanceIds: [] });
      use.count++;
      if (family.variant) use.variants[family.variant] = (use.variants[family.variant] ?? 0) + 1;
      if (use.instanceIds.length < 10) use.instanceIds.push(instance.id);
    }
    const links: ScreenLink[] = [];
    const reactive = [screen as SceneNode, ...screen.findAll((n) => 'reactions' in n && n.reactions.length > 0)];
    for (const node of reactive) {
      if (!('reactions' in node)) continue;
      for (const reaction of node.reactions) {
        for (const action of reactionActions(reaction)) {
          const link: ScreenLink = { fromId: node.id, from: node.name, trigger: reaction.trigger?.type ?? 'NONE', action: action.type };
          if (action.type === 'NODE') {
            link.action = action.navigation;
            if (action.destinationId) {
              link.toId = action.destinationId;
              if (!nameCache.has(action.destinationId)) {
                const dest = await figma.getNodeByIdAsync(action.destinationId);
                const destScreen = dest ? screenOf(dest) : null;
                nameCache.set(action.destinationId, dest ? { id: destScreen?.id, name: destScreen?.name } : null);
              }
              const target = nameCache.get(action.destinationId);
              if (target) {
                link.toScreenId = target.id;
                link.toScreen = target.name;
              } else link.broken = true;
            }
          } else if (action.type === 'URL') link.toScreen = action.url;
          links.push(link);
        }
      }
    }
    screens.push({
      id: screen.id,
      name: screen.name,
      pageId: page.id,
      page: page.name,
      width: round(screen.width),
      height: round(screen.height),
      components,
      links,
      status: screenStatus(screen),
    });
  }
  const graph = { id: page.id, name: page.name, flows: [...page.flowStartingPoints], screens };
  graphCache.set(page.id, graph);
  return graph;
}

async function designGraph(pageId?: unknown): Promise<PageGraph[]> {
  await figma.loadAllPagesAsync();
  watchChanges();
  const pages = figma.root.children.filter((p) => pageId === undefined || p.id === pageId || p.name === pageId);
  if (!pages.length) throw new Error(`Page ${pageId} not found`);
  const graphs = [];
  for (const page of pages) graphs.push(await buildPageGraph(page));
  return graphs;
}

function componentTotals(graphs: PageGraph[]) {
  const totals = new Map<string, { count: number; screens: Set<string>; variants: Record<string, number> }>();
  for (const screen of graphs.flatMap((g) => g.screens)) {
    for (const [family, use] of Object.entries(screen.components)) {
      const t = totals.get(family) ?? { count: 0, screens: new Set<string>(), variants: {} };
      t.count += use.count;
      t.screens.add(screen.id);
      for (const [v, c] of Object.entries(use.variants)) t.variants[v] = (t.variants[v] ?? 0) + c;
      totals.set(family, t);
    }
  }
  return totals;
}

// ---------- frame outline (what's inside a screen, for the mind map) ----------

const DECORATION = new Set(['RECTANGLE', 'ELLIPSE', 'VECTOR', 'LINE', 'STAR', 'POLYGON', 'BOOLEAN_OPERATION']);

// Meaningful structure only: frames/groups/instances/text/images/icons; plain decorative shapes are skipped.
function outlineNode(node: SceneNode, depth: number, maxChildren: number): Json | null {
  if (!node.visible) return null;
  const out: Json = { id: node.id, name: node.name };
  if (node.type === 'TEXT') return { ...out, kind: 'text', text: node.characters.slice(0, 60) };
  if (node.type === 'INSTANCE') return { ...out, kind: 'instance' };
  if (isIcon(node)) return { ...out, kind: 'icon' };
  if (DECORATION.has(node.type)) return hasImageFill(node) ? { ...out, kind: 'image' } : null;
  out.kind = hasImageFill(node) ? 'image' : 'frame';
  if ('children' in node) {
    const visible = node.children.filter((c) => c.visible);
    if (depth > 0) {
      const kids = visible.map((c) => outlineNode(c, depth - 1, maxChildren)).filter((c): c is Json => c !== null);
      if (kids.length) out.children = kids.slice(0, maxChildren);
      if (kids.length > maxChildren) out.more = kids.length - maxChildren;
    } else if (visible.length) {
      out.childCount = visible.length;
    }
  }
  return out;
}

// ---------- duplicate detection ----------

const DUP_CONTAINERS = new Set(['FRAME', 'GROUP']);

function shapeSignature(node: SceneNode, exact: boolean, depth: number): string {
  const parts: string[] = [node.type];
  if ('layoutMode' in node) parts.push(node.layoutMode);
  if (exact) {
    if ('layoutMode' in node && node.layoutMode !== 'NONE') {
      parts.push(`g${node.itemSpacing}p${node.paddingTop},${node.paddingRight},${node.paddingBottom},${node.paddingLeft}`);
    }
    if ('cornerRadius' in node && typeof node.cornerRadius === 'number') parts.push(`r${node.cornerRadius}`);
    if (node.type === 'TEXT' && node.fontSize !== figma.mixed) parts.push(`f${node.fontSize}`);
  }
  if (node.type === 'INSTANCE') parts.push(node.name);
  if ('children' in node && node.type !== 'INSTANCE' && depth > 0) {
    parts.push(`[${node.children.filter((c) => c.visible).map((c) => shapeSignature(c, exact, depth - 1)).join(',')}]`);
  }
  return parts.join(':');
}

function describeShape(node: SceneNode): Json {
  const out: Json = {};
  if ('layoutMode' in node && node.layoutMode !== 'NONE') {
    out.gap = node.itemSpacing;
    out.padding = [node.paddingTop, node.paddingRight, node.paddingBottom, node.paddingLeft];
  }
  if ('cornerRadius' in node && typeof node.cornerRadius === 'number') out.radius = node.cornerRadius;
  if ('findAll' in node) {
    const sizes = [...new Set(node.findAll((n) => n.type === 'TEXT').map((t) => ((t as TextNode).fontSize === figma.mixed ? 'mixed' : (t as TextNode).fontSize)))];
    if (sizes.length) out.fontSizes = sizes;
  }
  return out;
}

async function findDuplicates(scope: BaseNode & ChildrenMixin, minCount: number, limit: number) {
  const candidates = scope.findAll(
    (n) =>
      DUP_CONTAINERS.has(n.type) &&
      n.visible &&
      !insideInstance(n, scope) &&
      n.parent?.type !== 'PAGE' &&
      n.parent?.type !== 'SECTION' &&
      'findAll' in n &&
      n.findAll(() => true).length >= 2,
  ) as Array<FrameNode | GroupNode>;

  // Group by structure ignoring spacing/radius/font sizes; exact signatures then split each group
  // into identical copies (duplicate) vs near-copies with different values (inconsistent).
  const loose = new Map<string, Array<FrameNode | GroupNode>>();
  for (const n of candidates) {
    const key = shapeSignature(n, false, 6);
    if (!loose.has(key)) loose.set(key, []);
    (loose.get(key) as Array<FrameNode | GroupNode>).push(n);
  }

  const size = (n: SceneNode) => ('findAll' in n ? n.findAll(() => true).length : 1);
  const member = (n: SceneNode) => ({ id: n.id, name: n.name, screen: screenOf(n)?.name });
  const covered = new Set<string>();
  const isCovered = (n: BaseNode) => {
    for (let p: BaseNode | null = n; p; p = p.parent) if (covered.has(p.id)) return true;
    return false;
  };
  const results: Json[] = [];

  // Biggest structures first, so nested parts of an already-reported duplicate aren't repeated.
  const groups = [...loose.values()].filter((g) => g.length >= minCount).sort((a, b) => size(b[0]) * b.length - size(a[0]) * a.length);
  for (const group of groups) {
    const fresh = group.filter((n) => !isCovered(n));
    if (fresh.length < minCount) continue;
    const variants = new Map<string, Array<FrameNode | GroupNode>>();
    for (const n of fresh) {
      const key = shapeSignature(n, true, 6);
      if (!variants.has(key)) variants.set(key, []);
      (variants.get(key) as Array<FrameNode | GroupNode>).push(n);
    }
    const inconsistent = variants.size > 1;
    results.push({
      kind: inconsistent ? 'inconsistent' : 'duplicate',
      count: fresh.length,
      layers: size(fresh[0]),
      screens: new Set(fresh.map((n) => screenOf(n)?.id)).size,
      members: fresh.slice(0, 8).map(member),
      differences: inconsistent ? [...variants.values()].map((v) => ({ count: v.length, example: v[0].id, ...describeShape(v[0]) })) : undefined,
    });
    for (const n of fresh) covered.add(n.id);
    if (results.length >= limit) break;
  }
  return results;
}

// ---------- variables (create) ----------

async function getCollection(id: unknown): Promise<VariableCollection> {
  const collection = await figma.variables.getVariableCollectionByIdAsync(String(id));
  if (!collection) throw new Error(`Variable collection ${id} not found`);
  return collection;
}

function modeIdFor(collection: VariableCollection, mode: string): string {
  const found = collection.modes.find((m) => m.name === mode || m.modeId === mode);
  if (!found) {
    throw new Error(`Mode "${mode}" is not in "${collection.name}" (modes: ${collection.modes.map((m) => m.name).join(', ')})`);
  }
  return found.modeId;
}

// Aliases may name a variable by id or by name (same collection preferred).
async function findVariable(ref: string, collection: VariableCollection): Promise<Variable> {
  const byId = await figma.variables.getVariableByIdAsync(ref);
  if (byId) return byId;
  const all = await figma.variables.getLocalVariablesAsync();
  const match =
    all.find((v) => v.name === ref && v.variableCollectionId === collection.id) ?? all.find((v) => v.name === ref);
  if (!match) throw new Error(`Alias target "${ref}" not found`);
  return match;
}

async function toVariableValue(type: VariableResolvedDataType, value: unknown, collection: VariableCollection): Promise<VariableValue> {
  if (value && typeof value === 'object' && 'alias' in value) {
    return figma.variables.createVariableAlias(await findVariable(String((value as P).alias), collection));
  }
  if (type === 'COLOR') {
    if (typeof value !== 'string' || !/^#([0-9a-f]{6}|[0-9a-f]{8})$/i.test(value)) {
      throw new Error(`Color value must be a hex like #FF8866, got ${JSON.stringify(value)}`);
    }
    return rgba(value);
  }
  if (type === 'FLOAT') return num(value);
  if (type === 'BOOLEAN') return Boolean(value);
  return String(value);
}

// ---------- commands ----------

const handlers: Record<string, (p: P) => Promise<unknown>> = {
  // read
  // Cheap check used by the server before every write: is the file the AI confirmed still open?
  // ---- FigJam: stickies, shapes with text, connectors, whole diagrams, and reading a board back ----
  async create_sticky(p) {
    requireFigJam('create_sticky');
    const node = figma.createSticky();
    await setJamText(node.text, str(p.text));
    if (has(p, 'color')) node.fills = [{ type: 'SOLID', color: parseHex(str(p.color)).color }];
    if (has(p, 'wide')) node.isWideWidth = Boolean(p.wide);
    await placeJam(node, p);
    return serialize(node, 0);
  },

  async create_shape(p) {
    requireFigJam('create_shape');
    const node = await jamShape(p as JamNode);
    await placeJam(node, p);
    return serialize(node, 0);
  },

  async create_connector(p) {
    requireFigJam('create_connector');
    const node = await jamConnector(str(p.from), str(p.to), p as JamEdge);
    if (has(p, 'parentId')) (await getParent(p.parentId)).appendChild(node);
    markCreated(node);
    return serialize(node, 0);
  },

  // One call draws a laid-out diagram: sections (lanes), nodes, then connectors between them.
  // Every node keeps its diagram id in plugin data, so board_map can say which step it is.
  async figjam_draw(p) {
    requireFigJam('figjam_draw');
    const ids: Record<string, string> = {};
    const root = has(p, 'parentId') ? await getParent(p.parentId) : figma.currentPage;
    let frame: SectionNode | null = null;
    if (has(p, 'section')) {
      const s = p.section as { name: string; x?: number; y?: number; width: number; height: number; fill?: string };
      frame = figma.createSection();
      frame.name = s.name;
      // No position given: to the right of everything already on the page, so nothing is covered.
      const right = figma.currentPage.children.filter((c) => c !== frame).reduce((m, c) => Math.max(m, c.x + c.width), -Infinity);
      frame.x = s.x ?? (Number.isFinite(right) ? right + 200 : 0);
      frame.y = s.y ?? 0;
      frame.resizeWithoutConstraints(s.width, s.height);
      if (s.fill) frame.fills = [{ type: 'SOLID', color: parseHex(s.fill).color }];
      (root as ChildrenMixin).appendChild(frame);
      markCreated(frame);
      ids['$section'] = frame.id;
    }
    const parent = (frame ?? root) as BaseNode & ChildrenMixin;
    for (const lane of (p.lanes as JamLane[] | undefined) ?? []) {
      const band = figma.createShapeWithText();
      band.shapeType = 'SQUARE';
      band.resize(lane.width, lane.height);
      band.fills = [{ type: 'SOLID', color: parseHex(lane.fill ?? '#F8FAFC').color }];
      band.strokes = [{ type: 'SOLID', color: parseHex('#E2E8F0').color }];
      await setJamText(band.text, '', { size: 12 });
      parent.appendChild(band);
      band.x = lane.x; band.y = lane.y;
      band.name = `Lane/${lane.name}`;
      band.locked = true; // a background: dragging steps should not move it
      band.setPluginData('wara:lane', lane.name);
      markCreated(band);
      const label = figma.createText();
      await setJamText(label, lane.name, { size: 16, bold: true, color: '#475569' });
      parent.appendChild(label);
      label.x = lane.x + 16; label.y = lane.y + 12;
      label.name = `Lane label/${lane.name}`;
      label.locked = true;
      markCreated(label);
    }
    for (const n of (p.nodes as JamNode[]) ?? []) {
      const node = n.kind === 'sticky' ? figma.createSticky() : await jamShape(n);
      if (n.kind === 'sticky') {
        await setJamText((node as StickyNode).text, n.text);
        if (n.fill) (node as StickyNode).fills = [{ type: 'SOLID', color: parseHex(n.fill).color }];
      }
      parent.appendChild(node);
      node.x = n.x; node.y = n.y;
      if (n.id) { node.setPluginData('wara:id', n.id); ids[n.id] = node.id; }
      if (n.role) node.setPluginData('wara:role', n.role);
      node.name = n.name ?? `${n.role ?? n.kind ?? 'Node'}/${n.text.slice(0, 40)}`;
      markCreated(node);
    }
    for (const e of (p.edges as JamEdge[]) ?? []) {
      const from = e.fromPoint ? null : ids[e.from] ?? e.from, to = e.toPoint ? null : ids[e.to] ?? e.to;
      const c = await jamConnector(from, to, e, parent);
      if (e.id) c.setPluginData('wara:id', e.id);
      markCreated(c);
    }
    return { ids, section: frame ? serialize(frame, 0) : null };
  },

  // A compact read of a board (or one section): what each sticky/shape says, which connects to which,
  // grouped by section and lane, with the diagram ids written by figjam_draw.
  async board_map(p) {
    const scope = has(p, 'sectionId') ? await getParent(p.sectionId) : figma.currentPage;
    const out = { file: figma.root.name, editorType: figma.editorType, sections: [] as unknown[], loose: [] as unknown[] };
    const describe = (n: SceneNode) => {
      const base = { id: n.id, x: Math.round(n.x), y: Math.round(n.y), w: Math.round(n.width), h: Math.round(n.height), waraId: n.getPluginData('wara:id') || undefined, role: n.getPluginData('wara:role') || undefined };
      if (n.type === 'STICKY') return { ...base, kind: 'sticky', text: n.text.characters, color: hexOf(n.fills) };
      if (n.type === 'SHAPE_WITH_TEXT') return n.getPluginData('wara:lane') ? null : { ...base, kind: 'shape', shape: n.shapeType, text: n.text.characters, color: hexOf(n.fills) };
      if (n.type === 'CONNECTOR') {
        const end = (e: ConnectorEndpoint) => ('endpointNodeId' in e ? e.endpointNodeId : null);
        return { id: n.id, kind: 'connector', from: end(n.connectorStart), to: end(n.connectorEnd), label: n.text.characters || undefined, dashed: (n.dashPattern ?? []).length > 0 || undefined, waraId: n.getPluginData('wara:id') || undefined };
      }
      if (n.type === 'TEXT') return n.name.startsWith('Lane label/') ? null : { ...base, kind: 'text', text: n.characters };
      if (n.type === 'CODE_BLOCK') return { ...base, kind: 'code', text: n.code };
      return null;
    };
    const lanesOf = (s: BaseNode & ChildrenMixin) => s.children.filter((c) => 'getPluginData' in c && (c as SceneNode).getPluginData('wara:lane')).map((c) => ({ name: (c as SceneNode).getPluginData('wara:lane'), x: Math.round((c as SceneNode).x), y: Math.round((c as SceneNode).y), w: Math.round((c as SceneNode).width), h: Math.round((c as SceneNode).height) }));
    const children = 'children' in scope ? (scope as ChildrenMixin).children : [];
    for (const c of children) {
      if (c.type === 'SECTION') {
        out.sections.push({ id: c.id, name: c.name, lanes: lanesOf(c), items: c.children.map(describe).filter(Boolean) });
      } else {
        const d = describe(c);
        if (d) out.loose.push(d);
      }
    }
    return out;
  },

  // ---- Figma Motion (beta API): read the animation on a node and everything inside it ----
  async motion_read(p) {
    const motion = motionApi();
    const root = has(p, 'nodeId') ? await getSceneNode(p.nodeId) : figma.currentPage.selection[0];
    if (!root) throw new Error('Select a frame or layer with Motion animation, or pass nodeId');
    const nodes = [root as SceneNode, ...('findAll' in root ? (root as ChildrenMixin).findAll(() => true) : [])];
    const animated = [];
    for (const n of nodes) {
      if (!('animations' in n)) continue;
      const m = n as SceneNode & MotionNodeMixin;
      const tracks = readTracks(m.animations);
      const styles = m.animationStyles.map((s) => (s.type === 'FIGMA' ? { type: 'FIGMA', name: s.name, id: s.id, props: s.props } : { type: 'CUSTOM', name: s.name, id: s.id }));
      // parentId + nth (1-based among same-name siblings) let a Lottie export find the layer: Figma names the 2nd "Dot" "Dot_2"
      const nth = n.parent && 'children' in n.parent ? (n.parent as ChildrenMixin).children.filter((c) => c.name === n.name).indexOf(n as SceneNode) + 1 : 1;
      // path: the groups between the root and this layer (a Lottie export opens them so nested layers animate)
      const path: string[] = [];
      for (let p = n.parent; p && p.id !== root.id && p.type !== 'PAGE' && p.type !== 'DOCUMENT'; p = p.parent) path.unshift(p.name);
      if (tracks.length || styles.length) animated.push({ id: n.id, name: n.name, type: n.type, parentId: n.parent?.id ?? null, nth, path, hint: await motionHint(n as SceneNode), tracks, styles });
    }
    return {
      file: figma.root.name,
      root: { id: root.id, name: root.name, type: root.type },
      playhead: motion.playheadPosition ?? null,
      timelines: 'timelines' in root ? (root as SceneNode & MotionNodeMixin).timelines.map((t) => ({ id: t.id, duration: t.duration })) : [],
      animated,
      animationStyles: p.styles ? motion.figmaAnimationStyles().map((s) => ({ styleId: s.styleId, name: s.name, description: s.description, props: s.props })) : undefined,
    };
  },

  // Write Motion animation under a frame: keyframe tracks and/or animation styles (presets), each aimed
  // at layers by name (glob "Dot*", or "all"). The server has already turned the spec into Figma fields,
  // values and seconds; stagger shifts each matched layer by that many seconds.
  async motion_animate(p) {
    motionApi();
    const root = await getSceneNode(p.rootId);
    const all = [root, ...('findAll' in root ? (root as ChildrenMixin).findAll(() => true) : [])] as SceneNode[];
    const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9ก-๙*]/g, '');
    const match = (pattern: string) => {
      const pat = norm(pattern);
      if (pat === 'all' || pat === '*') return all.filter((n) => n !== root && n.parent === root);
      if (pat === 'root') return [root];
      const re = new RegExp(`^${pat.split('*').map((x) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*')}$`);
      return all.filter((n) => n.id === pattern || re.test(norm(n.name)));
    };
    const report: unknown[] = [];
    const missing: string[] = [];
    // Layer ids have been seen to change after a write (every child of the frame, not only the animated
    // ones). Record which step does it, so the cause is a fact rather than a guess.
    const idsNow = async () => { const r = await figma.getNodeByIdAsync(str(p.rootId)); return r && 'children' in r ? (r as ChildrenMixin).children.map((c) => c.id).join(',') : ''; };
    let lastIds = await idsNow();
    const idChanges: string[] = [];
    const idTrace = async (stage: string) => { const now = await idsNow(); if (now !== lastIds) { idChanges.push(stage); lastIds = now; } };
    if (p.replace) {
      // start clean: remove manual tracks and applied styles on every layer the items aim at
      for (const it of p.items as P[]) for (const n of match(str(it.layer))) {
        const m = n as SceneNode & MotionNodeMixin;
        if (!('animationStyles' in m)) continue;
        for (const s of m.animationStyles) m.removeAnimationStyle(s.id);
        for (const field of Object.keys(m.manualKeyframeTracks)) if (field !== 'fills' && field !== 'strokes' && field !== 'effects') m.removeManualKeyframeTrack({ type: 'PROPERTY', name: field as KeyframePropertyFieldName });
      }
      await idTrace('after replace (remove tracks/styles)');
    }
    for (const it of p.items as P[]) {
      const targets = match(str(it.layer)).filter((n) => 'applyManualKeyframeTrack' in n) as (SceneNode & MotionNodeMixin)[];
      if (!targets.length) { missing.push(str(it.layer)); continue; }
      targets.forEach((n, k) => {
        const shift = num(it.stagger ?? 0) * k;
        if (it.kind === 'style') {
          const cfg = { ...(it.config as P) };
          const id = n.applyAnimationStyle(str(it.styleId), { ...cfg, timelineOffset: num(cfg.timelineOffset ?? 0) + shift } as AnimationStyleConfiguration);
          report.push({ layer: n.name, id: n.id, style: it.styleId, applied: id });
        } else {
          // TRANSLATION_X/Y keys are offsets from where the layer sits (checked with export_node_image at a
          // playhead: absolute values put the layer at 2× its position), so the values go in as given
          const keys = (it.keys as P[]).map((key) => ({ timelinePosition: num(key.t) + shift, value: key.value as KeyframeValue, ...(key.easing ? { easing: key.easing as MotionEasing } : {}) }));
          n.applyManualKeyframeTrack(it.field as KeyframeField, { ...(it.base ? { baseValue: it.base as KeyframeValue } : {}), keyframes: keys });
          report.push({ layer: n.name, id: n.id, field: it.field, keys: keys.length });
        }
      });
      await idTrace(`after ${str(it.layer)} ${it.kind === 'style' ? str(it.styleId) : JSON.stringify(it.field)}`);
    }
    if (has(p, 'duration')) {
      const [timeline] = (root as SceneNode & MotionNodeMixin).timelines ?? [];
      if (timeline) (root as SceneNode & MotionNodeMixin).setTimelineDuration(timeline.id, num(p.duration));
      await idTrace('after setTimelineDuration');
    }
    return { applied: report, missing, layers: all.filter((n) => n.parent === root).map((n) => n.name), idChanges };
  },

  async file_name() {
    return { fileName: figma.root.name };
  },

  async get_document_info() {
    return {
      fileName: figma.root.name,
      editorType: figma.editorType,
      currentPage: { id: figma.currentPage.id, name: figma.currentPage.name },
      pages: figma.root.children.map((page) => ({ id: page.id, name: page.name })),
      topLevel: figma.currentPage.children.map((node) => serialize(node, 0)),
    };
  },

  async get_selection(p) {
    return figma.currentPage.selection.map((node) => serialize(node, num(p.depth)));
  },

  async get_node(p) {
    return serialize(await getNode(p.nodeId), num(p.depth));
  },

  async find_nodes(p) {
    const scope = has(p, 'withinId') ? await getNode(p.withinId) : figma.currentPage;
    if (!('findAll' in scope)) throw new Error(`Node ${p.withinId} has no children to search`);
    if (scope.type === 'DOCUMENT') await figma.loadAllPagesAsync();
    const query = has(p, 'name') ? str(p.name).toLowerCase() : null;
    const type = has(p, 'type') ? str(p.type) : null;
    const matches = scope.findAll(
      (n) => (!query || n.name.toLowerCase().includes(query)) && (!type || n.type === type),
    );
    return { total: matches.length, nodes: matches.slice(0, num(p.limit)).map((n) => serialize(n, 0)) };
  },

  async export_node_image(p) {
    const node = await getSceneNode(p.nodeId);
    const longest = Math.max(node.width, node.height, 1);
    const scale = has(p, 'maxSize') ? Math.min(num(p.scale), num(p.maxSize) / longest) : num(p.scale);
    const bytes = await node.exportAsync({ format: 'PNG', constraint: { type: 'SCALE', value: scale } });
    return { base64: figma.base64Encode(bytes) };
  },

  async get_styles() {
    const [paints, texts, effects] = await Promise.all([
      figma.getLocalPaintStylesAsync(),
      figma.getLocalTextStylesAsync(),
      figma.getLocalEffectStylesAsync(),
    ]);
    return {
      paint: paints.map((s) => ({ id: s.id, name: s.name, colors: solidHexes(s.paints) })),
      text: texts.map((s) => ({
        id: s.id,
        name: s.name,
        font: s.fontName,
        fontSize: s.fontSize,
        lineHeight: s.lineHeight,
      })),
      effect: effects.map((s) => ({ id: s.id, name: s.name, effects: s.effects.map((e) => e.type) })),
    };
  },

  async get_variables() {
    const [collections, variables] = await Promise.all([
      figma.variables.getLocalVariableCollectionsAsync(),
      figma.variables.getLocalVariablesAsync(),
    ]);
    return {
      collections: collections.map((c) => ({ id: c.id, name: c.name, modes: c.modes })),
      variables: variables.map((v) => ({
        id: v.id,
        name: v.name,
        collectionId: v.variableCollectionId,
        type: v.resolvedType,
        values: Object.fromEntries(Object.entries(v.valuesByMode).map(([mode, val]) => [mode, formatVariableValue(val)])),
      })),
    };
  },

  async get_components(p) {
    if (p.allPages) await figma.loadAllPagesAsync();
    const scope = p.allPages ? figma.root : figma.currentPage;
    const found = scope.findAllWithCriteria({ types: ['COMPONENT_SET', 'COMPONENT'] });
    return found
      .filter((n) => n.type === 'COMPONENT_SET' || n.parent?.type !== 'COMPONENT_SET')
      .map((n) => ({
        id: n.id,
        name: n.name,
        type: n.type,
        key: n.key,
        width: round(n.width),
        height: round(n.height),
        properties: Object.fromEntries(
          Object.entries(n.componentPropertyDefinitions).map(([name, def]) => [
            name,
            { type: def.type, default: def.defaultValue, options: def.variantOptions },
          ]),
        ),
        variants: n.type === 'COMPONENT_SET' ? n.children.map((c) => ({ id: c.id, name: c.name })) : undefined,
      }));
  },

  // create
  async create_frame(p) {
    const parent = await getParent(p.parentId);
    const frame = figma.createFrame();
    frame.name = str(p.name);
    frame.resize(num(p.width), num(p.height));
    frame.fills = has(p, 'fill') ? [solid(str(p.fill))] : [];
    if (has(p, 'clip')) frame.clipsContent = Boolean(p.clip);
    applyCorners(frame, p);
    applyAutoLayout(frame, p);
    if (frame.layoutMode !== 'NONE') {
      // Keep the requested size; widthSizing/heightSizing (applied in attach) can switch to HUG/FILL.
      frame.primaryAxisSizingMode = 'FIXED';
      frame.counterAxisSizingMode = 'FIXED';
      frame.resize(num(p.width), num(p.height));
    }
    attach(frame, parent, p);
    return serialize(frame, 0);
  },

  async create_rectangle(p) {
    const parent = await getParent(p.parentId);
    const rect = figma.createRectangle();
    rect.name = str(p.name);
    rect.resize(num(p.width), num(p.height));
    if (has(p, 'fill')) rect.fills = [solid(str(p.fill))];
    applyCorners(rect, p);
    attach(rect, parent, p);
    return serialize(rect, 0);
  },

  async create_ellipse(p) {
    const parent = await getParent(p.parentId);
    const ellipse = figma.createEllipse();
    ellipse.name = str(p.name);
    ellipse.resize(num(p.width), num(p.height));
    if (has(p, 'fill')) ellipse.fills = [solid(str(p.fill))];
    attach(ellipse, parent, p);
    return serialize(ellipse, 0);
  },

  async create_text(p) {
    const parent = await getParent(p.parentId);
    const font = { family: str(p.fontFamily), style: str(p.fontStyle) };
    await figma.loadFontAsync(font);
    const node = figma.createText();
    node.fontName = font;
    node.characters = str(p.text);
    node.name = has(p, 'name') ? str(p.name) : str(p.text).slice(0, 40);
    await applyText(node, { ...p, fontFamily: undefined, fontStyle: undefined });
    attach(node, parent, p);
    return serialize(node, 0);
  },

  async create_instance(p) {
    let component: ComponentNode;
    if (has(p, 'componentKey')) {
      component = await figma.importComponentByKeyAsync(str(p.componentKey));
    } else {
      const node = await getNode(p.componentId);
      if (node.type === 'COMPONENT_SET') component = node.defaultVariant;
      else if (node.type === 'COMPONENT') component = node;
      else throw new Error(`Node ${p.componentId} is ${node.type}, not a component`);
    }
    const parent = await getParent(p.parentId);
    const instance = component.createInstance();
    attach(instance, parent, p);
    if (has(p, 'properties')) instance.setProperties(p.properties as { [name: string]: string | boolean });
    return serialize(instance, 0);
  },

  async create_component(p) {
    const source = await getSceneNode(p.nodeId);
    const ours = createdByWara(source);
    const component = figma.createComponentFromNode(source);
    if (ours) markCreated(component);
    if (has(p, 'name')) component.name = str(p.name);
    if (has(p, 'description')) component.description = str(p.description);
    return serialize(component, 0);
  },

  // Components named "Type=Primary, State=Hover" become one set whose variant properties are Type and State.
  async combine_as_variants(p) {
    const nodes = await Promise.all((p.nodeIds as string[]).map((id) => getSceneNode(id)));
    const components = nodes.map((n) => {
      if (n.type !== 'COMPONENT') throw new Error(`Node ${n.id} is ${n.type}; turn it into a component first (create_component)`);
      return n;
    });
    const parent = has(p, 'parentId') ? await getParent(p.parentId) : (components[0].parent as BaseNode & ChildrenMixin);
    const set = figma.combineAsVariants(components, parent);
    if (components.every(createdByWara)) markCreated(set);
    set.name = str(p.name);
    if (p.layout !== false) {
      set.layoutMode = 'HORIZONTAL';
      set.layoutWrap = 'WRAP';
      set.primaryAxisSizingMode = 'FIXED';
      set.counterAxisSizingMode = 'AUTO';
      set.itemSpacing = num(p.gap ?? 16);
      set.counterAxisSpacing = num(p.gap ?? 16);
      set.paddingTop = set.paddingRight = set.paddingBottom = set.paddingLeft = num(p.padding ?? 24);
      set.resize(num(p.width ?? 960), set.height);
    }
    return { ...serialize(set, 0), variantProperties: set.componentPropertyDefinitions };
  },

  // A section around top-level nodes (screens of one flow). Its name carries the review status: "Teacher · Draft".
  async create_section(p) {
    const page = figma.currentPage;
    const nodes = await Promise.all(((p.nodeIds as string[] | undefined) ?? []).map((id) => getSceneNode(id)));
    for (const n of nodes) if (n.parent !== page) throw new Error(`${n.name} is not at the top level of the current page`);
    const pad = num(p.padding ?? 80);
    const section = figma.createSection();
    if (nodes.every(createdByWara)) markCreated(section);
    // Asked straight for "· In review" / "· Approved": start as Draft, promote only if the prototype check passes.
    const wanted = str(p.name);
    section.name = parseSectionTitle(wanted).prototype && REVIEW_SUFFIX.test(wanted.trim()) ? wanted.trim().replace(REVIEW_SUFFIX, '$1Draft') : wanted;
    if (nodes.length) {
      const minX = Math.min(...nodes.map((n) => n.x));
      const minY = Math.min(...nodes.map((n) => n.y));
      const maxX = Math.max(...nodes.map((n) => n.x + n.width));
      const maxY = Math.max(...nodes.map((n) => n.y + n.height));
      section.x = minX - pad;
      section.y = minY - pad;
      section.resizeWithoutConstraints(maxX - minX + pad * 2, maxY - minY + pad * 2);
      for (const n of nodes) {
        const x = n.x - section.x;
        const y = n.y - section.y;
        section.appendChild(n);
        n.x = x;
        n.y = y;
      }
    } else {
      section.x = num(p.x ?? 0);
      section.y = num(p.y ?? 0);
      section.resizeWithoutConstraints(num(p.width ?? 1600), num(p.height ?? 1000));
    }
    if (section.name !== wanted) {
      await guardName(section, wanted, p); // throws, leaving the section as Draft
      section.name = wanted;
    }
    return { id: section.id, name: section.name, x: section.x, y: section.y, width: section.width, height: section.height, children: nodes.map((n) => n.name) };
  },

  // TEXT / BOOLEAN / INSTANCE_SWAP property on a component (or a variant's set), bound to layers inside it.
  async add_component_property(p) {
    let target = await getSceneNode(p.componentId);
    if (target.type === 'COMPONENT' && target.parent?.type === 'COMPONENT_SET') target = target.parent;
    if (target.type !== 'COMPONENT' && target.type !== 'COMPONENT_SET') throw new Error(`Node ${target.id} is ${target.type}, not a component`);
    const type = str(p.type) as 'TEXT' | 'BOOLEAN' | 'INSTANCE_SWAP';
    const key = target.addComponentProperty(str(p.name), type, p.defaultValue as string | boolean);
    const field = type === 'TEXT' ? 'characters' : type === 'BOOLEAN' ? 'visible' : 'mainComponent';
    const bound: string[] = [];
    for (const id of (p.bindNodeIds as string[] | undefined) ?? []) {
      const layer = await getSceneNode(id);
      if (type === 'TEXT' && layer.type !== 'TEXT') throw new Error(`${layer.name} is not a text layer`);
      if (type === 'INSTANCE_SWAP' && layer.type !== 'INSTANCE') throw new Error(`${layer.name} is not an instance`);
      layer.componentPropertyReferences = { ...(layer.componentPropertyReferences ?? {}), [field]: key };
      bound.push(layer.name);
    }
    return { componentId: target.id, property: key, bound };
  },

  async create_paint_style(p) {
    const style = figma.createPaintStyle();
    style.name = str(p.name);
    style.paints = [solid(str(p.color))];
    return { id: style.id, name: style.name };
  },

  async create_text_style(p) {
    const font = { family: str(p.fontFamily), style: str(p.fontStyle) };
    await figma.loadFontAsync(font);
    const style = figma.createTextStyle();
    style.name = str(p.name);
    style.fontName = font;
    style.fontSize = num(p.fontSize);
    if (has(p, 'lineHeight')) style.lineHeight = lineHeight(p.lineHeight);
    if (has(p, 'letterSpacing')) style.letterSpacing = letterSpacing(p.letterSpacing);
    return { id: style.id, name: style.name };
  },

  async duplicate_node(p) {
    const node = await getSceneNode(p.nodeId);
    const copy = node.clone();
    markCreated(copy);
    if (has(p, 'parentId')) (await getParent(p.parentId)).appendChild(copy);
    if (has(p, 'x')) copy.x = num(p.x);
    if (has(p, 'y')) copy.y = num(p.y);
    if (has(p, 'name')) copy.name = str(p.name);
    return serialize(copy, 0);
  },

  async group_nodes(p) {
    const nodes = await Promise.all((p.nodeIds as string[]).map((id) => getSceneNode(id)));
    const parent = nodes[0].parent;
    if (!parent || !('appendChild' in parent)) throw new Error('First node has no parent to group into');
    const group = figma.group(nodes, parent);
    if (nodes.every(createdByWara)) markCreated(group);
    if (has(p, 'name')) group.name = str(p.name);
    return serialize(group, 0);
  },

  async ungroup_node(p) {
    const node = await getSceneNode(p.nodeId);
    if (node.type !== 'GROUP') throw new Error(`Node ${p.nodeId} is ${node.type}, not GROUP`);
    return figma.ungroup(node).map((n) => serialize(n, 0));
  },

  // modify
  async set_text(p) {
    const node = await getText(p.nodeId);
    await loadTextFonts(node);
    node.characters = str(p.text);
    return serialize(node, 0);
  },

  async set_text_props(p) {
    const node = await getText(p.nodeId);
    await loadTextFonts(node);
    await applyText(node, p);
    return serialize(node, 0);
  },

  async set_fill(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('fills' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no fills`);
    node.fills = p.color === 'none' ? [] : [solid(str(p.color))];
    return serialize(node, 0);
  },

  async set_stroke(p) {
    return setStroke(p);
  },

  async set_corner_radius(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('cornerRadius' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no corner radius`);
    applyCorners(node, p);
    return serialize(node, 0);
  },

  async set_effects(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('effects' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) cannot have effects`);
    const effects: Effect[] = [];
    for (const s of (p.shadows as P[] | undefined) ?? []) {
      const base = {
        color: rgba(str(s.color)),
        offset: { x: num(s.x), y: num(s.y) },
        radius: num(s.blur),
        spread: num(s.spread),
        visible: true,
        blendMode: 'NORMAL' as const,
      };
      effects.push(s.inner ? { ...base, type: 'INNER_SHADOW' } : { ...base, type: 'DROP_SHADOW' });
    }
    if (has(p, 'layerBlur')) effects.push({ type: 'LAYER_BLUR', blurType: 'NORMAL', radius: num(p.layerBlur), visible: true });
    if (has(p, 'backgroundBlur')) {
      effects.push({ type: 'BACKGROUND_BLUR', blurType: 'NORMAL', radius: num(p.backgroundBlur), visible: true });
    }
    node.effects = effects;
    return serialize(node, 0);
  },

  async set_props(p) {
    const node = await getSceneNode(p.nodeId);
    if (has(p, 'name')) await guardName(node, str(p.name), p);
    if (has(p, 'name')) node.name = str(p.name);
    if (has(p, 'visible')) node.visible = Boolean(p.visible);
    if (has(p, 'locked')) node.locked = Boolean(p.locked);
    if (has(p, 'opacity') && 'opacity' in node) node.opacity = num(p.opacity);
    if (has(p, 'clip') && 'clipsContent' in node) node.clipsContent = Boolean(p.clip);
    if (has(p, 'absolute') && 'layoutPositioning' in node) node.layoutPositioning = p.absolute ? 'ABSOLUTE' : 'AUTO';
    if (has(p, 'rotation')) {
      if (!('rotation' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) cannot be rotated`);
      rotateAroundCenter(node as SceneNode & LayoutMixin, num(p.rotation));
    }
    return serialize(node, 0);
  },

  async set_auto_layout(p) {
    const node = await getAutoLayoutNode(p.nodeId);
    applyAutoLayout(node, p);
    applySizing(node, p);
    return serialize(node, 0);
  },

  async set_sizing(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('layoutSizingHorizontal' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no layout sizing`);
    applySizing(node, p);
    return serialize(node, 0);
  },

  async move_resize(p) {
    const node = await getSceneNode(p.nodeId);
    if (has(p, 'x')) node.x = num(p.x);
    if (has(p, 'y')) node.y = num(p.y);
    if (has(p, 'width') || has(p, 'height')) {
      if (!('resize' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) cannot be resized`);
      node.resize(has(p, 'width') ? num(p.width) : node.width, has(p, 'height') ? num(p.height) : node.height);
    }
    return serialize(node, 0);
  },

  async move_to_parent(p) {
    const node = await getSceneNode(p.nodeId);
    const parent = await getParent(p.parentId);
    if (has(p, 'index')) parent.insertChild(num(p.index), node);
    else parent.appendChild(node);
    return serialize(node, 0);
  },

  async rename_node(p) {
    const node = await getSceneNode(p.nodeId);
    await guardName(node, str(p.name), p);
    node.name = str(p.name);
    return serialize(node, 0);
  },

  async delete_node(p) {
    const node = await getSceneNode(p.nodeId);
    if (!createdByWara(node) && !p.confirm) {
      throw new Error(`"${node.name}" (${node.type}) was not made through wridge, so it is the user's work. Ask the user; pass confirm: true only after they agree to delete it.`);
    }
    node.remove();
    return { deleted: p.nodeId };
  },

  async select_nodes(p) {
    const nodes = await Promise.all((p.nodeIds as string[]).map((id) => getSceneNode(id)));
    figma.currentPage.selection = nodes;
    if (p.zoom !== false) figma.viewport.scrollAndZoomIntoView(nodes);
    return { selected: nodes.map((n) => n.id) };
  },

  // images
  async set_image_fill(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('fills' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) cannot hold an image`);
    const image = figma.createImage(figma.base64Decode(str(p.base64)));
    node.fills = [{ type: 'IMAGE', imageHash: image.hash, scaleMode: p.scaleMode as ImagePaint['scaleMode'] }];
    const { width, height } = await image.getSizeAsync();
    return { ...serialize(node, 0), imageSize: { width, height } };
  },

  async copy_fills(p) {
    const source = await getSceneNode(p.fromNodeId);
    if (!('fills' in source) || !Array.isArray(source.fills)) {
      throw new Error(`Node ${p.fromNodeId} has no copyable fills`);
    }
    const updated = [];
    for (const id of p.toNodeIds as string[]) {
      const target = await getSceneNode(id);
      if (!('fills' in target)) throw new Error(`Node ${id} (${target.type}) cannot hold fills`);
      target.fills = source.fills;
      updated.push(serialize(target, 0));
    }
    return updated;
  },

  // design → code
  async get_design_context(p) {
    const node = await getSceneNode(p.nodeId);
    const unapproved = p.userApproved ? null : approvalProblem(node);
    if (unapproved) throw new Error(unapproved);
    const ctx = new CodeContext(Boolean(p.expandInstances));
    const root = await codeNode(node, num(p.depth), ctx, true);
    return {
      root,
      tokens: ctx.usedTokens,
      components: [...ctx.components.values()],
      fonts: [...ctx.fonts],
      assets: ctx.assets,
    };
  },

  async export_assets(p) {
    const out = [];
    for (const item of p.items as { nodeId: string; format: 'auto' | 'svg' | 'png' | 'pdf'; scale: number }[]) {
      const node = await getSceneNode(item.nodeId);
      const format = item.format === 'auto' ? (isIcon(node) ? 'svg' : 'png') : item.format;
      const bytes =
        format === 'pdf'
          ? await node.exportAsync({ format: 'PDF' }) // vector, text stays selectable: one document page per frame
          : format === 'svg'
          ? await node.exportAsync({ format: 'SVG', svgIdAttribute: true }) // layer names as ids: create_lottie animates by name
          : await node.exportAsync({ format: 'PNG', constraint: { type: 'SCALE', value: item.scale } });
      out.push({ nodeId: node.id, name: node.name, format, base64: figma.base64Encode(bytes) });
    }
    return out;
  },

  // Tokens from a Foundations page (free-plan friendly) and/or the file's variables + styles.
  async get_tokens(p) {
    const source = has(p, 'source') ? str(p.source) : 'auto';
    const foundations = source === 'file' ? null : await readFoundations();
    if (source === 'foundations' && !foundations) throw new Error('No page named "Foundations" in this file');
    const file = source === 'foundations' ? null : ((await handlers.get_file_tokens({})) as TokenData & { file: string });
    const parts: TokenData[] = [];
    if (foundations) parts.push(foundations);
    if (file) parts.push(file);
    const merge = (key: keyof TokenData) => parts.flatMap((d) => d[key]);
    return {
      file: figma.root.name,
      sources: [foundations && `page:${foundations.page}`, file && 'variables+styles'].filter(Boolean),
      collections: merge('collections'),
      variables: merge('variables'),
      paintStyles: merge('paintStyles'),
      textStyles: merge('textStyles'),
      effectStyles: merge('effectStyles'),
    };
  },

  async scan_design_values(p) {
    return scanValues(await getSceneNode(p.nodeId), num(p.limit));
  },

  async get_file_tokens() {
    const [collections, variables, paints, texts, effects] = await Promise.all([
      figma.variables.getLocalVariableCollectionsAsync(),
      figma.variables.getLocalVariablesAsync(),
      figma.getLocalPaintStylesAsync(),
      figma.getLocalTextStylesAsync(),
      figma.getLocalEffectStylesAsync(),
    ]);
    const names = new Map(variables.map((v) => [v.id, tokenName(v.name, v.id)]));
    const modeNames = new Map(collections.flatMap((c) => c.modes.map((m) => [m.modeId, m.name] as const)));
    const ctx = new CodeContext(false);
    return {
      file: figma.root.name,
      collections: collections.map((c) => ({
        name: c.name,
        defaultMode: modeNames.get(c.defaultModeId),
        modes: c.modes.map((m) => m.name),
      })),
      variables: variables.map((v) => ({
        token: names.get(v.id),
        name: v.name,
        collection: collections.find((c) => c.id === v.variableCollectionId)?.name,
        type: v.resolvedType,
        values: Object.fromEntries(
          Object.entries(v.valuesByMode).map(([mode, value]) => [modeNames.get(mode) ?? mode, variableValue(value, names)]),
        ),
      })),
      paintStyles: await Promise.all(
        paints.map(async (s) => ({ token: tokenName(s.name, s.id), name: s.name, value: await paintsValue(s.paints, undefined, ctx) })),
      ),
      textStyles: texts.map((s) => ({
        token: tokenName(s.name, s.id),
        name: s.name,
        family: s.fontName.family,
        style: s.fontName.style,
        size: s.fontSize,
        lineHeight: lineHeightValue(s.lineHeight),
        letterSpacing: letterSpacingValue(s.letterSpacing),
      })),
      effectStyles: effects.map((s) => ({
        token: tokenName(s.name, s.id),
        name: s.name,
        shadows: s.effects
          .filter((e): e is DropShadowEffect | InnerShadowEffect => e.type === 'DROP_SHADOW' || e.type === 'INNER_SHADOW')
          .map((e) => ({
            x: e.offset.x,
            y: e.offset.y,
            blur: e.radius,
            spread: e.spread ?? 0,
            color: toHex(e.color, e.color.a),
            inset: e.type === 'INNER_SHADOW',
          })),
      })),
    };
  },

  // shapes
  async create_polygon(p) {
    const parent = await getParent(p.parentId);
    const polygon = figma.createPolygon();
    polygon.name = str(p.name);
    polygon.pointCount = num(p.pointCount);
    polygon.resize(num(p.width), num(p.height));
    if (has(p, 'fill')) polygon.fills = [solid(str(p.fill))];
    attach(polygon, parent, p);
    return serialize(polygon, 0);
  },

  async create_star(p) {
    const parent = await getParent(p.parentId);
    const star = figma.createStar();
    star.name = str(p.name);
    star.pointCount = num(p.pointCount);
    star.innerRadius = num(p.innerRadius);
    star.resize(num(p.width), num(p.height));
    if (has(p, 'fill')) star.fills = [solid(str(p.fill))];
    attach(star, parent, p);
    return serialize(star, 0);
  },

  async create_line(p) {
    const parent = await getParent(p.parentId);
    const line = figma.createLine();
    line.name = str(p.name);
    line.resize(num(p.length), 0);
    line.strokes = [solid(str(p.color))];
    line.strokeWeight = num(p.weight);
    if (has(p, 'cap')) line.strokeCap = p.cap as StrokeCap;
    attach(line, parent, p);
    if (has(p, 'rotation')) line.rotation = num(p.rotation);
    return serialize(line, 0);
  },

  async create_from_svg(p) {
    const parent = await getParent(p.parentId);
    const frame = figma.createNodeFromSvg(str(p.svg));
    frame.name = str(p.name);
    if (has(p, 'width')) frame.rescale(num(p.width) / frame.width);
    if (has(p, 'color')) recolor(frame, str(p.color));
    attach(frame, parent, p);
    return serialize(frame, 1);
  },

  async boolean_shapes(p) {
    const { nodes, parent, index } = await siblings(p.nodeIds);
    let result: BooleanOperationNode;
    switch (p.operation) {
      case 'UNION':
        result = figma.union(nodes, parent, index);
        break;
      case 'SUBTRACT':
        result = figma.subtract(nodes, parent, index);
        break;
      case 'INTERSECT':
        result = figma.intersect(nodes, parent, index);
        break;
      default:
        result = figma.exclude(nodes, parent, index);
    }
    if (has(p, 'name')) result.name = str(p.name);
    return serialize(result, 1);
  },

  async flatten(p) {
    const { nodes, parent, index } = await siblings(p.nodeIds);
    const vector = figma.flatten(nodes, parent, index);
    if (has(p, 'name')) vector.name = str(p.name);
    return serialize(vector, 0);
  },

  async outline_stroke(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('outlineStroke' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no stroke to outline`);
    const outlined = node.outlineStroke();
    if (!outlined) throw new Error(`Node ${p.nodeId} has no visible stroke`);
    const parent = node.parent;
    if (parent && outlined.parent !== parent) parent.insertChild(parent.children.indexOf(node) + 1, outlined);
    outlined.name = has(p, 'name') ? str(p.name) : `${node.name} (outlined)`;
    if (p.removeOriginal) node.remove();
    return serialize(outlined, 0);
  },

  async vector_read(p) {
    const node = await getSceneNode(p.nodeId);
    if (node.type === 'VECTOR') return describeNet(node, netOf(node));
    if (!TO_VECTOR.has(node.type)) throw new Error(`${node.name} (${node.type}) has no points — vectors, rectangles, ellipses, polygons, stars and lines do`);
    // read a shape's points without turning it into a vector: flatten a copy, read, throw the copy away
    const copy = node.clone();
    try {
      const v = figma.flatten([copy], node.parent ?? figma.currentPage);
      const out = describeNet(v, netOf(v));
      v.remove();
      return { ...out, id: node.id, type: node.type, note: 'Editing turns it into a vector (vector_edit / bend_shape return the new id)' };
    } catch (e) {
      if (!copy.removed) copy.remove();
      throw e;
    }
  },

  async vector_edit(p) {
    const node = await editableVector(p.nodeId);
    const net = netOf(node);
    (p.ops as P[]).forEach((op, i) => {
      try { applyOp(net, op); } catch (e) { throw new Error(`ops[${i}] (${String(op.op)}): ${e instanceof Error ? e.message : String(e)}`); }
    });
    await saveNet(node, net);
    return describeNet(node, netOf(node));
  },

  async bend_shape(p) {
    const node = await editableVector(p.nodeId);
    const net = netOf(node);
    bend(net, p);
    await saveNet(node, net);
    const out = describeNet(node, netOf(node));
    return { id: out.id, name: out.name, width: out.width, height: out.height, points: out.points.length, segments: out.segments.length };
  },

  async use_as_mask(p) {
    const mask = await getSceneNode(p.maskNodeId);
    if (!('isMask' in mask)) throw new Error(`Node ${p.maskNodeId} (${mask.type}) cannot be a mask`);
    const { nodes, parent, index } = await siblings([mask.id, ...(p.nodeIds as string[])]);
    const group = figma.group(nodes, parent, index);
    // The mask clips everything above it, so it must be the group's bottom child.
    group.insertChild(0, mask);
    mask.isMask = true;
    mask.maskType = p.maskType as MaskType;
    group.name = has(p, 'name') ? str(p.name) : 'Mask group';
    return serialize(group, 1);
  },

  async remove_mask(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('isMask' in node) || !node.isMask) throw new Error(`Node ${p.nodeId} is not a mask`);
    node.isMask = false;
    return serialize(node, 0);
  },

  // prototype
  async get_prototype(p) {
    const scope = has(p, 'nodeId') ? await getNode(p.nodeId) : figma.currentPage;
    if (scope.type === 'DOCUMENT') throw new Error('Pass a page or a layer id, not the document');
    const hasReactions = (n: BaseNode): n is SceneNode & ReactionMixin => 'reactions' in n && n.reactions.length > 0;
    const nodes: Array<SceneNode & ReactionMixin> = [];
    if (hasReactions(scope)) nodes.push(scope);
    if ('findAll' in scope) nodes.push(...scope.findAll(hasReactions).filter(hasReactions));
    const limit = num(p.limit);
    return {
      flows: figma.currentPage.flowStartingPoints,
      total: nodes.length,
      interactions: await Promise.all(nodes.slice(0, limit).map(describeReactions)),
    };
  },

  async add_interaction(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('reactions' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) cannot have interactions`);
    const kind = str(p.action);
    let action: Action;
    if (kind === 'BACK' || kind === 'CLOSE') {
      action = { type: kind };
    } else if (kind === 'URL') {
      action = { type: 'URL', url: str(p.url) };
    } else {
      const destination = await getSceneNode(p.destinationId);
      action = {
        type: 'NODE',
        destinationId: destination.id,
        navigation: kind as Navigation,
        transition: buildTransition(p),
        preserveScrollPosition: Boolean(p.preserveScroll),
      };
    }
    const trigger = buildTrigger(p);
    const kept = node.reactions.filter((r) => !(p.replace !== false && r.trigger?.type === trigger.type));
    await node.setReactionsAsync([...kept, { trigger, actions: [action] }]);
    return describeReactions(node);
  },

  async remove_interactions(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('reactions' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no interactions`);
    const before = node.reactions.length;
    const kept = has(p, 'trigger') ? node.reactions.filter((r) => r.trigger?.type !== p.trigger) : [];
    await node.setReactionsAsync(kept);
    return { nodeId: node.id, removed: before - kept.length, remaining: kept.length };
  },

  async set_flow_start(p) {
    const node = await getSceneNode(p.nodeId);
    // A screen sits on the page directly or inside (nested) sections — both can start a flow.
    let page: BaseNode | null = node.parent;
    while (page && page.type === 'SECTION') page = page.parent;
    if (!page || page.type !== 'PAGE') throw new Error('A flow must start on a screen: a frame directly on the page or inside a section');
    const others = page.flowStartingPoints.filter((f) => f.nodeId !== node.id);
    page.flowStartingPoints = p.remove
      ? others
      : [...others, { nodeId: node.id, name: has(p, 'name') ? str(p.name) : node.name }];
    return { page: page.name, flows: page.flowStartingPoints };
  },

  async set_scroll(p) {
    const node = await getSceneNode(p.nodeId);
    if (!('overflowDirection' in node)) throw new Error(`Node ${p.nodeId} (${node.type}) is not a frame`);
    if (has(p, 'overflow')) node.overflowDirection = p.overflow as OverflowDirection;
    if (has(p, 'fixedNodeIds')) {
      const ids = p.fixedNodeIds as string[];
      for (const id of ids) {
        const child = await getSceneNode(id);
        if (child.parent !== node) throw new Error(`Node ${id} is not a direct child of ${p.nodeId}`);
        // Fixed layers are the topmost section of the frame's children.
        node.appendChild(child);
      }
      node.numberOfFixedChildren = ids.length;
    }
    return serialize(node, 0);
  },

  // design system
  async apply_style(p) {
    const node = await getSceneNode(p.nodeId);
    if (has(p, 'fillStyleId')) {
      if (!('setFillStyleIdAsync' in node)) throw new Error(`Node ${p.nodeId} cannot take a fill style`);
      await node.setFillStyleIdAsync(str(p.fillStyleId));
    }
    if (has(p, 'strokeStyleId')) {
      if (!('setStrokeStyleIdAsync' in node)) throw new Error(`Node ${p.nodeId} cannot take a stroke style`);
      await node.setStrokeStyleIdAsync(str(p.strokeStyleId));
    }
    if (has(p, 'effectStyleId')) {
      if (!('setEffectStyleIdAsync' in node)) throw new Error(`Node ${p.nodeId} cannot take an effect style`);
      await node.setEffectStyleIdAsync(str(p.effectStyleId));
    }
    if (has(p, 'textStyleId')) {
      if (node.type !== 'TEXT') throw new Error(`Node ${p.nodeId} is ${node.type}, not TEXT`);
      const style = await figma.getStyleByIdAsync(str(p.textStyleId));
      if (!style || style.type !== 'TEXT') throw new Error(`Text style ${p.textStyleId} not found`);
      await loadTextFonts(node);
      await figma.loadFontAsync(style.fontName);
      await node.setTextStyleIdAsync(style.id);
    }
    return serialize(node, 0);
  },

  async bind_variable(p) {
    const node = await getSceneNode(p.nodeId);
    const variable = await figma.variables.getVariableByIdAsync(str(p.variableId));
    if (!variable) throw new Error(`Variable ${p.variableId} not found`);
    const field = str(p.field);
    if (field === 'fill' || field === 'stroke') {
      const key = field === 'fill' ? 'fills' : 'strokes';
      if (!(key in node)) throw new Error(`Node ${p.nodeId} (${node.type}) has no ${key}`);
      const target = node as SceneNode & MinimalFillsMixin & MinimalStrokesMixin;
      const current = key === 'fills' ? target.fills : target.strokes;
      const first = Array.isArray(current) ? (current as Paint[]).find((x): x is SolidPaint => x.type === 'SOLID') : undefined;
      const bound = figma.variables.setBoundVariableForPaint(first ?? solid('#000000'), 'color', variable);
      if (key === 'fills') target.fills = [bound];
      else target.strokes = [bound];
    } else {
      if (node.type === 'TEXT') await loadTextFonts(node);
      node.setBoundVariable(field as VariableBindableNodeField, variable);
    }
    return serialize(node, 0);
  },

  // design graph
  async design_map(p) {
    if (p.refresh) graphCache.clear();
    const graphs = await designGraph(p.pageId);
    const totals = componentTotals(graphs);
    const local = figma.root.findAllWithCriteria({ types: ['COMPONENT_SET', 'COMPONENT'] }).filter((c) => c.parent?.type !== 'COMPONENT_SET');
    const breakpoints: Record<string, Record<string, string>> = {};
    for (const s of graphs.flatMap((g) => g.screens)) {
      const m = /^(.*?)\s*\/\s*(mobile|tablet|desktop)$/i.exec(s.name);
      if (m) (breakpoints[m[1]] ??= {})[m[2].toLowerCase()] = s.id;
    }
    return {
      file: figma.root.name,
      foundations: (await findFoundations()) !== null,
      pages: graphs.map((g) => ({
        id: g.id,
        name: g.name,
        flows: g.flows,
        screens: g.screens.map((s) => ({
          id: s.id,
          name: s.name,
          size: `${s.width}×${s.height}`,
          status: s.status,
          components: Object.entries(s.components)
            .sort((a, b) => b[1].count - a[1].count)
            .map(([name, use]) => [name, use.count]),
          to: [...new Set(s.links.filter((l) => l.toScreenId && l.toScreenId !== s.id).map((l) => l.toScreen))],
          toIds: [...new Set(s.links.filter((l) => l.toScreenId && l.toScreenId !== s.id).map((l) => l.toScreenId as string))],
        })),
      })),
      breakpoints,
      components: [...totals].sort((a, b) => b[1].count - a[1].count).map(([name, t]) => [name, t.count, t.screens.size]),
      unusedComponents: local.filter((c) => !totals.has(c.name)).map((c) => c.name),
      // Slides, Storyboard… sections: real work, but not part of the (Prototype) design graph.
      otherWork: figma.root.children.filter((pg) => p.pageId === undefined || pg.id === p.pageId || pg.name === p.pageId).flatMap((pg) => otherSections(pg)),
    };
  },

  async frame_outline(p) {
    return outlineNode(await getSceneNode(p.nodeId), num(p.depth), num(p.maxChildren));
  },

  async component_usage(p) {
    const graphs = await designGraph(p.pageId);
    const screens = graphs.flatMap((g) => g.screens);
    if (has(p, 'screenId')) {
      const screen = screens.find((s) => s.id === p.screenId || s.name === p.screenId);
      if (!screen) throw new Error(`Screen ${p.screenId} not found`);
      return { screen: screen.name, components: screen.components };
    }
    const totals = componentTotals(graphs);
    if (!has(p, 'component')) {
      return [...totals].sort((a, b) => b[1].count - a[1].count).map(([name, t]) => ({ name, count: t.count, screens: t.screens.size, variants: t.variants }));
    }
    const query = str(p.component).toLowerCase();
    const matches = [...totals.keys()].filter((name) => name.toLowerCase() === query || name.toLowerCase().includes(query));
    return matches.map((name) => ({
      name,
      variants: (totals.get(name) as { variants: Record<string, number> }).variants,
      usedIn: screens
        .filter((s) => s.components[name])
        .map((s) => ({ screen: s.name, screenId: s.id, page: s.page, count: s.components[name].count, instanceIds: s.components[name].instanceIds })),
    }));
  },

  async find_duplicates(p) {
    const scope = has(p, 'scopeId') ? await getNode(p.scopeId) : figma.currentPage;
    if (!('findAll' in scope) || scope.type === 'DOCUMENT') throw new Error('scopeId must be a page or a frame');
    if (scope.type === 'PAGE') await scope.loadAsync();
    return findDuplicates(scope, num(p.minCount), num(p.limit));
  },

  async check_flows(p) {
    const graphs = await designGraph(p.pageId);
    const screens = graphs.flatMap((g) => g.screens);
    const byId = new Map(screens.map((s) => [s.id, s]));
    const incoming = new Map<string, number>();
    for (const s of screens) for (const l of s.links) if (l.toScreenId && l.toScreenId !== s.id) incoming.set(l.toScreenId, (incoming.get(l.toScreenId) ?? 0) + 1);

    const reached = new Set<string>();
    const queue = graphs.flatMap((g) => g.flows.map((f) => f.nodeId)).filter((id) => byId.has(id));
    while (queue.length) {
      const id = queue.shift() as string;
      if (reached.has(id)) continue;
      reached.add(id);
      // A link can lead outside the prototype screens (another section, a component page): not walked.
      for (const l of (byId.get(id) as ScreenInfo).links) if (l.toScreenId && byId.has(l.toScreenId) && !reached.has(l.toScreenId)) queue.push(l.toScreenId);
    }

    const ACTION_NAME = /button|btn|cta|tab|nav|link|menu|back|close/i;
    const unlinkedActions = [];
    for (const s of screens) {
      const linked = new Set(s.links.map((l) => l.fromId));
      for (const [family, use] of Object.entries(s.components)) {
        if (!ACTION_NAME.test(family)) continue;
        for (const id of use.instanceIds) {
          const node = await figma.getNodeByIdAsync(id);
          const hasLink = node && (linked.has(id) || ('findAll' in node && node.findAll((n) => linked.has(n.id)).length > 0));
          if (!hasLink) unlinkedActions.push({ screen: s.name, component: family, id });
        }
      }
    }
    const leaves = (s: ScreenInfo) => s.links.some((l) => (l.toScreenId && l.toScreenId !== s.id) || l.action === 'BACK' || l.action === 'CLOSE');
    return {
      flows: graphs.flatMap((g) => g.flows),
      deadEnds: screens.filter((s) => (incoming.get(s.id) ?? 0) > 0 && !leaves(s)).map((s) => ({ id: s.id, name: s.name })),
      orphans: screens.filter((s) => !incoming.has(s.id) && !reached.has(s.id) && !graphs.some((g) => g.flows.some((f) => f.nodeId === s.id))).map((s) => ({ id: s.id, name: s.name })),
      unreachableFromFlows: graphs.some((g) => g.flows.length) ? screens.filter((s) => !reached.has(s.id) && incoming.has(s.id)).map((s) => s.name) : [],
      brokenLinks: screens.flatMap((s) => s.links.filter((l) => l.broken).map((l) => ({ screen: s.name, from: l.from, fromId: l.fromId }))),
      unlinkedActions: unlinkedActions.slice(0, num(p.limit)),
      unlinkedActionsTotal: unlinkedActions.length,
    };
  },

  async create_variable_collection(p) {
    const modes = p.modes as string[];
    const collection = figma.variables.createVariableCollection(str(p.name));
    collection.renameMode(collection.modes[0].modeId, modes[0]);
    for (const mode of modes.slice(1)) {
      try {
        collection.addMode(mode);
      } catch (err) {
        throw new Error(
          `${err instanceof Error ? err.message : err} — collection "${collection.name}" (${collection.id}) was created with modes: ${collection.modes.map((m) => m.name).join(', ')}. The Figma plan of this file limits modes.`,
        );
      }
    }
    return { id: collection.id, name: collection.name, modes: collection.modes.map((m) => m.name) };
  },

  async create_variables(p) {
    const collection = await getCollection(p.collectionId);
    const created = [];
    for (const spec of p.variables as P[]) {
      const variable = figma.variables.createVariable(str(spec.name), collection, spec.type as VariableResolvedDataType);
      for (const [mode, value] of Object.entries(spec.values as P)) {
        variable.setValueForMode(modeIdFor(collection, mode), await toVariableValue(variable.resolvedType, value, collection));
      }
      if (has(spec, 'scopes')) variable.scopes = spec.scopes as VariableScope[];
      created.push({ id: variable.id, name: variable.name, token: tokenName(variable.name, variable.id) });
    }
    return created;
  },

  async set_variable_value(p) {
    const variable = await figma.variables.getVariableByIdAsync(str(p.variableId));
    if (!variable) throw new Error(`Variable ${p.variableId} not found`);
    const collection = await getCollection(variable.variableCollectionId);
    variable.setValueForMode(modeIdFor(collection, str(p.mode)), await toVariableValue(variable.resolvedType, p.value, collection));
    return { id: variable.id, name: variable.name };
  },

  async set_instance_properties(p) {
    const node = await getSceneNode(p.nodeId);
    if (node.type !== 'INSTANCE') throw new Error(`Node ${p.nodeId} is ${node.type}, not INSTANCE`);
    node.setProperties(p.properties as { [name: string]: string | boolean });
    return serialize(node, 0);
  },
};

figma.ui.onmessage = async (msg: Command | UiMessage) => {
  if ('type' in msg) {
    if (msg.type === 'open' && /^http:\/\/localhost:\d+\/?$/.test(msg.url)) figma.openExternal(msg.url);
    if (msg.type === 'close') figma.closePlugin();
    if (msg.type === 'resize') figma.ui.resize(300, Math.max(160, Math.min(420, Math.round(msg.height))));
    return;
  }
  const handler = handlers[msg.command];
  try {
    if (!handler) throw new Error(`Unknown command "${msg.command}" — re-run the plugin in Figma to load the latest version`);
    figma.ui.postMessage({ id: msg.id, result: await handler(msg.params ?? {}) });
  } catch (err) {
    figma.ui.postMessage({ id: msg.id, error: err instanceof Error ? err.message : String(err) });
  }
};
