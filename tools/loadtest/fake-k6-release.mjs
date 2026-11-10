// A fake "latest k6 release" on 127.0.0.1:4720 for Settings → Load testing → Download k6
// (the server reads it when WRIDGE_K6_RELEASES=http://127.0.0.1:4720/releases/latest).
// The zip holds a stand-in k6.exe (a copy of Windows' whoami.exe) plus ~8 MB of padding, sent slowly
// so the progress bar shows. GET /bad switches the checksum to a wrong one (the download must be refused),
// GET /good switches it back.
import { createServer } from "node:http";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { crc32 } from "node:zlib";

const TAG = "v9.9.9";
const NAME = `k6-${TAG}-windows-amd64.zip`;
const BASE = "http://127.0.0.1:4720";

// a zip with stored (uncompressed) entries — enough for wridge's reader
function zip(files) {
  const local = [], central = [];
  let offset = 0;
  for (const [name, data] of files) {
    const n = Buffer.from(name);
    const crc = crc32(data);
    const h = Buffer.alloc(30);
    h.writeUInt32LE(0x04034b50, 0); h.writeUInt16LE(20, 4); h.writeUInt16LE(0x0800, 6); h.writeUInt16LE(0, 8);
    h.writeUInt32LE(crc, 14); h.writeUInt32LE(data.length, 18); h.writeUInt32LE(data.length, 22); h.writeUInt16LE(n.length, 26);
    local.push(h, n, data);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0); c.writeUInt16LE(20, 4); c.writeUInt16LE(20, 6); c.writeUInt16LE(0x0800, 8);
    c.writeUInt32LE(crc, 16); c.writeUInt32LE(data.length, 20); c.writeUInt32LE(data.length, 24); c.writeUInt16LE(n.length, 28);
    c.writeUInt32LE(offset, 42);
    central.push(c, n);
    offset += 30 + n.length + data.length;
  }
  const cd = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(files.length, 8); end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, cd, end]);
}

const pad = Buffer.alloc(8 << 20, "k6 padding so the progress bar has something to show\n");
const body = zip([
  [`k6-${TAG}-windows-amd64/k6.exe`, readFileSync("C:/Windows/System32/whoami.exe")],
  [`k6-${TAG}-windows-amd64/LICENSE.md`, Buffer.from("GNU AFFERO GENERAL PUBLIC LICENSE (stand-in for the test)\n")],
  [`k6-${TAG}-windows-amd64/padding.txt`, pad],
]);
const good = createHash("sha256").update(body).digest("hex");
let bad = false;

createServer((req, res) => {
  const url = req.url ?? "/";
  if (url === "/bad" || url === "/good") { bad = url === "/bad"; res.end(bad ? "checksum is now wrong\n" : "checksum is right again\n"); return; }
  if (url === "/releases/latest") {
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ tag_name: TAG, assets: [
      { name: `k6-${TAG}-windows-amd64.msi`, browser_download_url: `${BASE}/dl/k6-${TAG}-windows-amd64.msi` },
      { name: NAME, browser_download_url: `${BASE}/dl/${NAME}` },
      { name: `k6-${TAG}-checksums.txt`, browser_download_url: `${BASE}/dl/k6-${TAG}-checksums.txt` },
    ] }));
    return;
  }
  if (url === `/dl/k6-${TAG}-checksums.txt`) {
    res.end(`${"0".repeat(64)}  k6-${TAG}-windows-amd64.msi\n${bad ? "f".repeat(64) : good}  ${NAME}\n`);
    return;
  }
  if (url === `/dl/${NAME}`) {
    res.writeHead(200, { "content-type": "application/zip", "content-length": body.length });
    let at = 0;
    const step = () => {
      if (at >= body.length) { res.end(); return; }
      res.write(body.subarray(at, at + (256 << 10)));
      at += 256 << 10;
      setTimeout(step, 60);
    };
    step();
    return;
  }
  res.writeHead(404).end("not found");
}).listen(4720, "127.0.0.1", () => console.log(`fake k6 release on ${BASE} (${(body.length / 1048576).toFixed(1)} MB zip)`));
