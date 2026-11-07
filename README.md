# wridge

**Wisdom Bridge** — a Windows desktop app that bridges your AI, Figma and code, and keeps a brain of what was learned and agreed.

The secret meaning: **w**ara-b**ridge**, the project it grew out of.

**Website:** https://warayutkub.github.io/wridge · **Download:** [latest release](https://github.com/Warayutkub/wridge/releases/latest)

## What it does

- **Figma for your AI.** Claude Code (or any MCP client) reads and edits your Figma files through the wridge plugin: screens, components, prototypes, motion, PDFs.
- **Several AIs at once.** Each AI session works in its own Figma file; commands to one file take turns, different files run side by side.
- **A brain per project.** Notes, decisions and handoffs that survive across AI sessions, plus a code graph of the project.
- **One window.** Start and stop the server, pick the project and the Figma file, see what your AI is doing.

Your files, notes and code stay on your computer; the server only listens on localhost.

## Install

1. Download `wridge_x.y.z_x64-setup.exe` from [Releases](https://github.com/Warayutkub/wridge/releases) and run it. Windows may warn that the publisher is unknown: choose **More info → Run anyway**.
2. Open wridge (the cat in the tray) and press **Start server**.
3. In Figma desktop: **Plugins → Development → Import plugin from manifest…**, pick `manifest.json` from the folder shown in wridge → Settings → Figma plugin, then run **Plugins → Development → wridge** in your file.
4. In wridge → Settings → Connect an AI, press **Set up Claude Code**.

## Repository

- `src-tauri/` the desktop app (Tauri)
- `server/` the server in Rust: `app` (HTTP, MCP, plugin bridge), `figma` (design-side tools), `graph` (code graph, brain, memory)
- `figma-plugin/` the Figma plugin
- `web/` the app's pages
- `legacy-node/` the original Node server, kept as a fallback engine and as the reference for the parity tests
- `skills/` the methods the AI loads before each kind of work; `AI_GUIDE.md` the guide it reads first
- `site/` the website (plain HTML, CSS and JS; published by `.github/workflows/pages.yml`); `tools/site/` makes its demo project and screenshots

Building needs Rust, Node.js and the Dart SDK (for the Dart parser: `dart pub get` and `dart compile exe` in `legacy-node/codegraph/dart`). The parity tests read a few real projects listed in `server/parity-projects.json` (copy `server/parity-projects.example.json`).

## License

Free to use; not to be modified or redistributed. See [LICENSE](LICENSE).
