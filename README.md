# ponchi

A local-only, hand-drawn-style diagram generator for aligning AI agents and humans.

An AI agent writes a scene as declarative JSON; ponchi renders it as a hand-drawn
(Rough.js-style) sketch — a *ponchi-e* (ぽんち絵, "rough sketch") — so the essence
of a long explanation becomes something a human can grasp and judge at a glance.
Everything stays on the local machine: there is **no network access at runtime**.

The name comes from *ponchi-e*: extracting the essential shape of an idea by
sketching it roughly.

## Why

Agents are good at producing detailed prose. But for an unfamiliar area, a human
reviewer often wants a visual foothold first. A rough sketch strips a wall of text
down to its structure. ponchi gives the agent a way to draw that sketch — and to
*see its own output* and refine it — without sending anything to a third party.

## Features

- **Local-only**: the live-viewer server binds `127.0.0.1` only; `POST /api/scene`
  requires a Bearer token generated at startup. The generated SVG never contains
  external references (`url()`, file, or http URLs).
- **Hand-drawn look**: shapes are drawn with [`roughr`](https://crates.io/crates/roughr)
  (a Rust port of Rough.js), the same sketchy aesthetic as Excalidraw.
- **Pure Rust, no browser**: render pipeline is `roughr` → SVG → PNG via
  `usvg`/`resvg`/`tiny-skia`. No headless browser, no Cairo, no runtime HTTP client.
- **Agent-friendly schema**: describe nodes and edges; with auto-layout you don't
  write coordinates at all.
- **Auto-layout**: a `layout` block lets [`dagre`](https://crates.io/crates/dagre)
  (Sugiyama-style) place nodes, route edges, and position labels automatically.
- **CJK ready**: the embedded handwriting font (Yomogi, OFL) renders Japanese labels.
- **Same view for human and agent**: both look at the identical server-rendered PNG.

## Build

```sh
cargo build --release
```

The Yomogi font is embedded into the binary, so `target/release/ponchi` is a
self-contained executable — you can copy it anywhere and it keeps its hand-drawn
look without shipping `assets/fonts` alongside it. To make additional font
families selectable, pass `--font-path <dir>` and select one with
`--font-family <name>`.

## Usage

Write a scene (see the schema below) to a file, then:

```sh
# One-shot render to a file
cargo run --release -- render scene.json -o out/diagram.png

# Live viewer (omit --port to get an OS-assigned free port, printed at startup)
cargo run --release -- serve
# -> prints the URL and a Bearer token

# Push a scene (the viewer auto-refreshes when the version changes)
curl -X POST http://127.0.0.1:<port>/api/scene \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  --data-binary @scene.json
```

## Server endpoints (`serve`)

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Live viewer (polls and shows the PNG) |
| GET | `/api/scene.png` | Current scene as PNG (shared by human and agent) |
| GET | `/api/scene.svg` | Current scene as SVG |
| GET | `/api/version` | Scene version number |
| POST | `/api/scene` | Replace the scene (Bearer token required) |

## Auto-layout

Add a `layout` block and write only nodes (label / role / shape) and edges
(relations) — no coordinates. The engine assigns positions, edge routes, and
label placement. Without a `layout` block the scene uses manual coordinates
(fully backward compatible).

```jsonc
{
  "schema_version": "ponchi.scene/v0",
  "layout": {
    "mode": "auto",        // present => auto; absent => manual coordinates
    "engine": "layered",
    "direction": "LR",     // "LR" (left-to-right) or "TB" (top-to-bottom)
    "node_sep": 55,
    "rank_sep": 90,
    "margin": 30,
    "fit": "grow"          // "grow": size canvas from content / "fixed": use given canvas
  },
  "nodes": [
    { "id": "draft",  "label": "Draft",     "shape": "rect",    "role": "process" },
    { "id": "review", "label": "Review",    "shape": "rect",    "role": "input" },
    { "id": "done",   "label": "Published", "shape": "ellipse", "role": "persistence" }
  ],
  "edges": [
    { "id": "e1", "from": "draft",  "to": "review", "label": "submit" },
    { "id": "e2", "from": "review", "to": "done",   "label": "approve" }
  ]
}
```

Labels may be any text, including CJK (e.g. `"label": "下書き\n作成中"`); the
embedded Yomogi font renders them in the hand-drawn style.

- Node `x`/`y` must **not** be set in auto mode (it is rejected); `w`/`h` are
  optional overrides — otherwise size is derived from the label.
- `label` supports `\n` for multiple lines (all modes).
- Edge routes and label positions are decided by the engine in auto mode.
- v1 limitation: auto-layout places `nodes` and `edges` only. `zones` and `texts`
  are manual overlays (coordinates required); with `fit: "grow"` the canvas grows
  to include them so annotations are not clipped.

## Using ponchi from an AI agent

A [Claude Code](https://docs.claude.com/en/docs/claude-code) skill lives in
[`skills/ponchi/`](skills/ponchi/SKILL.md). Copy it into your skills directory:

```sh
cp -r skills/ponchi ~/.claude/skills/ponchi
```

Then an agent can drive ponchi for you: it starts the live viewer (which you
watch in the browser), writes a scene, pushes it, and reads back the rendered
PNG to self-check and refine — so you and the agent are looking at the same
picture while a design takes shape. The skill is self-contained; it explains
setup, the serve loop, and the scene format on its own.

## Design notes

- **Local-only**: server binds `127.0.0.1` only; `POST /api/scene` needs a
  startup-generated Bearer token; generated SVG carries no external references.
- **Rendering**: `roughr` hand-drawn shapes → SVG → PNG (`usvg`/`resvg`/`tiny-skia`),
  all pure Rust.
- **Auto-layout**: input is parsed into a geometry-optional type and resolved into a
  render-ready scene; `dagre` computes node positions, edge routes, and label spots.
- **Shared view**: human (browser) and agent (self-check) read the same
  resvg-rendered PNG, so there is no font/anti-aliasing drift between them.

## License

ponchi is dual-licensed under either of

- MIT license (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.

The embedded Yomogi font is separately licensed under the SIL Open Font License
1.1 (`assets/fonts/Yomogi-OFL.txt`); it is not covered by the licenses above.
