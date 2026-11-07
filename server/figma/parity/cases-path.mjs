// Node's path module (win32 on Windows, posix elsewhere) against the crate's node_path port: these decide
// "inside the project" checks and the paths listLottie reports.
export function pathCases({ repo }) {
  const cases = [];
  const add = (fn, ...args) => cases.push({ module: "path", fn, args });
  const bases = [repo, "C:\\", "C:/Users/someone/", "\\\\server\\share\\dir", "relative\\dir", "", "."];
  const rels = ["", ".", "..", "a/b/../c", "../../x", "C:\\Windows", "c:/windows/../x", "D:foo", "\\\\host\\share", "\\\\.\\pipe\\x", "//?/C:/x", "a\\\\b//c\\", "./a/./b/.", "...", "a/..", "Ünï/ไทย/../ok.json", "/rooted/x"];
  for (const b of bases) for (const r of rels) add("resolve", b, r);
  for (const p of rels.concat(bases)) { add("normalize", p); add("isAbsolute", p); }
  for (const b of bases) for (const r of rels) add("join", b, r);
  const abs = [repo, `${repo}\\server`, `${repo}\\SERVER\\x.json`, "C:\\", "C:\\Users", "c:\\users\\someone", "D:\\x", "\\\\server\\share\\a", "\\\\server\\share", `${repo}\\..\\other`, `${repo}2`];
  for (const a of abs) for (const b of abs) add("relative", a, b);
  return cases;
}
