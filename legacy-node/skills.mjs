// Skills are markdown files in /skills with a small front matter:
//   ---
//   name: ui-visual
//   description: When to load it (one line — shown to the AI in the skill list).
//   area: design | code | both (optional — design sessions list design skills, code sessions code skills)
//   ---
// They are read on every request, so adding or editing a file needs no restart.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

function parse(text, file) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  const meta = {};
  if (match) {
    for (const line of match[1].split(/\r?\n/)) {
      const i = line.indexOf(":");
      if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
  }
  return {
    name: meta.name || file.replace(/\.md$/, ""),
    description: meta.description || "",
    area: meta.area || "", // design | code | both — which sessions list it
    body: match ? text.slice(match[0].length).trim() : text.trim(),
    file,
    tokens: estimateTokens(text),
  };
}

// Rough token count for display (~4 characters per token for Latin text, ~1.5 for Thai and symbols),
// rounded to tens — close enough to compare skills, not a billing number.
export function estimateTokens(text) {
  let ascii = 0;
  let other = 0;
  for (const ch of text) {
    if (ch.charCodeAt(0) < 128) ascii++;
    else other++;
  }
  return Math.max(10, Math.round((ascii / 4 + other / 1.5) / 10) * 10);
}

export function loadSkills(dir) {
  let files;
  try {
    files = readdirSync(dir).filter((f) => f.endsWith(".md"));
  } catch {
    return [];
  }
  return files.map((f) => parse(readFileSync(join(dir, f), "utf8"), f)).sort((a, b) => a.name.localeCompare(b.name));
}

export function skillIndex(skills) {
  return skills.map((s) => `- \`${s.name}\`: ${s.description}`).join("\n");
}
