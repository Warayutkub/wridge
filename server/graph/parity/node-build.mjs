// Runs the Node code graph (legacy-node/codegraph/build.mjs) on one folder and prints the info JSON.
// usage: node node-build.mjs <absolute project dir> [--force]
import { updateGraph } from "../../../legacy-node/codegraph/build.mjs";

const [dir, ...flags] = process.argv.slice(2);
try {
  const info = await updateGraph(dir, { force: flags.includes("--force") });
  console.log(JSON.stringify(info));
} catch (err) {
  console.error(String(err?.message ?? err));
  process.exit(1);
}
