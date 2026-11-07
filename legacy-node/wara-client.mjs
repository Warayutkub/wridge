// Tiny client for scripts that need loops/logic beyond run_batch:
//   import { call, batch } from "file:///C:/path/to/wridge/legacy-node/wara-client.mjs";
//   const card = await call("create_frame", { name: "Card", parentId, widthSizing: "FILL" });
//   await call("create_text", { parentId: card.id, text: "Hello" });

const BASE = `http://127.0.0.1:${process.env.WARA_HTTP_PORT ?? 3156}`;

export async function call(tool, args = {}) {
  let res;
  try {
    res = await fetch(`${BASE}/api/tool`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: tool, args }),
    });
  } catch {
    throw new Error(`wridge server is not running on ${BASE} — ask the user to start it`);
  }
  const result = await res.json();
  const first = result.content?.[0];
  if (result.isError) throw new Error(`${tool}: ${first?.text ?? "failed"}`);
  if (first?.type === "image") return { base64: first.data };
  try {
    return JSON.parse(first?.text ?? "null");
  } catch {
    return first?.text;
  }
}

export const batch = (steps, stopOnError = true) => call("run_batch", { steps, stopOnError });
