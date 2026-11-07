// Researched guidance per project: <project>/.wara/knowledge/<topic>/<tool>@<major>.md
// The AI searches the web once for how the project's tools (at their versions) handle a topic
// (security, cache…), saves a short note with sources here, and reuses it until it goes stale.
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { projectRoot } from "./build.mjs";
import { WARA_DIR, ensureGitignored } from "./files.mjs";

const MAX_AGE_DAYS = 180;
const DIR = "knowledge";

// Packages worth a note: frameworks, data access, auth, validation. Everything else is noise here.
const NOTABLE = [
  /^next$/, /^react$/, /^express$/, /^fastify$/, /^hono$/, /^@nestjs\/core$/, /^next-auth$/, /^@auth\//, /^better-auth$/, /^lucia$/,
  /^iron-session$/, /^jose$/, /^jsonwebtoken$/, /^bcrypt(js)?$/, /^argon2$/, /^zod$/, /^valibot$/, /^yup$/,
  /^prisma$/, /^@prisma\/client$/, /^drizzle-orm$/, /^mysql2$/, /^pg$/, /^postgres$/, /^mongoose$/, /^mongodb$/, /^kysely$/, /^typeorm$/, /^sequelize$/,
  /^ioredis$/, /^redis$/, /^@upstash\//, /^bullmq$/, /^stripe$/, /^@line\//, /^nodemailer$/, /^multer$/, /^helmet$/,
  /github\.com\/gofiber\/fiber/, /github\.com\/gin-gonic\/gin/, /github\.com\/labstack\/echo/, /github\.com\/go-chi\/chi/,
  /gorm\.io\/gorm/, /github\.com\/jmoiron\/sqlx/, /github\.com\/jackc\/pgx/, /github\.com\/go-sql-driver\/mysql/,
  /github\.com\/golang-jwt\/jwt/, /github\.com\/redis\/go-redis/, /github\.com\/go-playground\/validator/, /golang\.org\/x\/crypto/,
];

export const major = (version) => /(\d+)/.exec(String(version ?? ""))?.[1] ?? "x";
const slug = (s) => String(s).toLowerCase().replace(/^github\.com\//, "").replace(/[^a-z0-9.@-]+/g, "-").replace(/^-+|-+$/g, "");
const readJson = (file) => {
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch {
    return null;
  }
};

// Folders holding a package.json or go.mod: the root and one level down (frontend/, backend/, apps/*).
function manifestDirs(root) {
  const dirs = [root];
  const children = (dir) => {
    try {
      return readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory() && !/^(\.|node_modules$|vendor$|dist$|build$)/.test(e.name)).map((e) => join(dir, e.name));
    } catch {
      return [];
    }
  };
  for (const d of children(root)) {
    dirs.push(d);
    if (/^(apps|packages|services)$/.test(d.split(/[\\/]/).pop())) dirs.push(...children(d));
  }
  return dirs.filter((d) => existsSync(join(d, "package.json")) || existsSync(join(d, "go.mod")));
}

/** The project's notable tools with installed (or declared) versions. */
export function detectStack(root) {
  const stack = new Map();
  for (const dir of manifestDirs(root)) {
    const rel = dir === root ? "." : dir.slice(root.length + 1).replaceAll("\\", "/");
    const pkg = readJson(join(dir, "package.json"));
    if (pkg) {
      for (const [name, range] of Object.entries({ ...pkg.devDependencies, ...pkg.dependencies })) {
        if (!NOTABLE.some((re) => re.test(name))) continue;
        const installed = readJson(join(dir, "node_modules", name, "package.json"))?.version ?? readJson(join(root, "node_modules", name, "package.json"))?.version;
        stack.set(name, { tool: name, version: installed ?? String(range).replace(/^[\^~>=<\s]+/, ""), lang: "js", where: rel });
      }
    }
    if (existsSync(join(dir, "go.mod"))) {
      const mod = readFileSync(join(dir, "go.mod"), "utf8");
      const go = /^go\s+(\S+)/m.exec(mod)?.[1];
      if (go) stack.set("go", { tool: "go", version: go, lang: "go", where: rel });
      for (const m of mod.matchAll(/^\s*(?:require\s+)?([\w.\-/]+\.[\w.\-/]+)\s+(v[\w.\-+]+)/gm)) {
        if (!NOTABLE.some((re) => re.test(m[1]))) continue;
        const tool = m[1].replace(/\/v\d+$/, "");
        stack.set(tool, { tool, version: m[2], lang: "go", where: rel });
      }
    }
  }
  return [...stack.values()];
}

const knowledgeDir = (root) => join(root, WARA_DIR, DIR);
const fileFor = (root, topic, tool, version) => join(knowledgeDir(root), slug(topic), `${slug(tool)}@${major(version)}.md`);

function parse(file) {
  const raw = readFileSync(file, "utf8");
  const m = /^<!--\s*([\s\S]*?)-->\n?/.exec(raw);
  let meta = {};
  try {
    meta = m ? JSON.parse(m[1]) : {};
  } catch {}
  return { meta, content: m ? raw.slice(m[0].length) : raw };
}

/** Saved notes, each marked stale when old or when the project's major version moved on. */
export function listKnowledge(projectDir, topic) {
  const root = projectRoot(projectDir);
  const stack = detectStack(root);
  const current = new Map(stack.map((s) => [slug(s.tool), major(s.version)]));
  const notes = [];
  const base = knowledgeDir(root);
  const topics = existsSync(base) ? readdirSync(base, { withFileTypes: true }).filter((e) => e.isDirectory() && (!topic || e.name === slug(topic))).map((e) => e.name) : [];
  for (const t of topics) {
    for (const f of readdirSync(join(base, t)).filter((n) => n.endsWith(".md"))) {
      const { meta } = parse(join(base, t, f));
      const [tool, ver] = f.slice(0, -3).split("@");
      const ageDays = Math.floor((Date.now() - Date.parse(meta.savedAt ?? statSync(join(base, t, f)).mtime)) / 86_400_000);
      const now = current.get(tool);
      const stale = ageDays > MAX_AGE_DAYS ? `older than ${MAX_AGE_DAYS} days` : now && now !== ver ? `project now uses major ${now}` : null;
      notes.push({ topic: t, tool: meta.tool ?? tool, version: meta.version ?? ver, savedAt: meta.savedAt ?? null, ageDays, stale });
    }
  }
  const covered = new Set(notes.filter((n) => !n.stale && (!topic || n.topic === slug(topic))).map((n) => slug(n.tool)));
  return {
    stack,
    facts: readJson(join(base, "facts.json")) ?? {},
    notes,
    missing: topic ? stack.filter((s) => !covered.has(slug(s.tool))).map((s) => `${s.tool}@${major(s.version)}`) : undefined,
  };
}

export function getKnowledge(projectDir, topic, tool) {
  const root = projectRoot(projectDir);
  const version = detectStack(root).find((s) => slug(s.tool) === slug(tool))?.version;
  const dir = join(knowledgeDir(root), slug(topic));
  let file = version ? fileFor(root, topic, tool, version) : null;
  if (!file || !existsSync(file)) {
    // Fall back to the newest note for this tool (any version), flagged so the caller can re-check.
    const any = existsSync(dir) ? readdirSync(dir).filter((n) => n.startsWith(`${slug(tool)}@`)).sort().pop() : null;
    if (!any) return { found: false, hint: `No ${topic} note for ${tool}. Research it (official docs first), then save it with action "save".` };
    file = join(dir, any);
  }
  const { meta, content } = parse(file);
  const ageDays = Math.floor((Date.now() - Date.parse(meta.savedAt)) / 86_400_000);
  const versionMoved = version && major(version) !== major(meta.version);
  return { found: true, ...meta, ageDays, stale: ageDays > MAX_AGE_DAYS || versionMoved || false, projectVersion: version ?? null, content };
}

// Project facts only the user can answer (deploy target, Redis available, CI provider…).
// Asked once per project, then reused: <project>/.wara/knowledge/facts.json
const factsFile = (root) => join(knowledgeDir(root), "facts.json");

export function readFacts(projectDir) {
  return readJson(factsFile(projectRoot(projectDir))) ?? {};
}

export function setFact(projectDir, key, value) {
  const root = projectRoot(projectDir);
  const gitignore = ensureGitignored(root);
  const facts = readJson(factsFile(root)) ?? {};
  if (value === "") delete facts[key];
  else facts[key] = { value, savedAt: new Date().toISOString() };
  mkdirSync(knowledgeDir(root), { recursive: true });
  writeFileSync(factsFile(root), `${JSON.stringify(facts, null, 2)}\n`);
  return { facts, gitignore: gitignore.added ? `added ${gitignore.pattern} to ${gitignore.gitignore}` : undefined };
}

export function saveKnowledge(projectDir, { topic, tool, version, content, sources }) {
  const root = projectRoot(projectDir);
  const detected = detectStack(root).find((s) => slug(s.tool) === slug(tool))?.version;
  const ver = version ?? detected;
  if (!ver) throw new Error(`Unknown version of ${tool} — pass version`);
  const gitignore = ensureGitignored(root); // .wara/ never goes into git
  const file = fileFor(root, topic, tool, ver);
  mkdirSync(join(knowledgeDir(root), slug(topic)), { recursive: true });
  const meta = { topic: slug(topic), tool, version: ver, savedAt: new Date().toISOString(), sources };
  writeFileSync(file, `<!-- ${JSON.stringify(meta)} -->\n${content.trim()}\n`);
  return { file: file.replaceAll("\\", "/"), ...meta, gitignore: gitignore.added ? `added ${gitignore.pattern} to ${gitignore.gitignore}` : undefined };
}
