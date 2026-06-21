# CLAUDE.md

Guidance for AI agents working on ponchi. This focuses on the things you **cannot**
read off the code: why ponchi exists, which constraints are non-negotiable, and the
traps that look like cleanup opportunities but aren't.

## Why ponchi exists (the motivation behind every decision)

ponchi turns a detailed explanation into a rough hand-drawn sketch (*ponchi-e*) so a
human can grasp the essential structure and make a judgment, instead of wading
through prose. It is a tool for **aligning an AI agent and a human**.

The agent is a first-class *user*, not just the author: it writes the scene, renders
it, and **reads the resulting PNG to check and refine its own work**. So whenever you
weigh a design choice, optimize for "easy and reliable for an agent to drive":
declarative input, automatic layout, deterministic output.

## The hard constraint: local-only (this is the reason the project exists)

Everything stays on the local machine. **No network egress at runtime — ever.** This
is the whole point: ponchi exists instead of a hosted/third-party diagram tool so the
content an agent draws (which may encode sensitive reasoning) never leaves the device.

Concretely, this forbids:
- any runtime HTTP-client dependency;
- binding the server anywhere but `127.0.0.1`;
- emitting external references (`url()`, `file:`, `http:`) in generated SVG;
- downloading anything at runtime (the font is **embedded in the binary**, not fetched).

Guardrail: for the native crates (`cargo tree -p ponchi -p ponchi-core`) the
dependency tree must show **no** HTTP-client crate (reqwest / hyper / tokio /
ureq / curl / wasm-bindgen / web-sys / js-sys). A new dependency that drags one in is
a red flag — find another way. `ponchi-wasm` legitimately uses `wasm-bindgen` and is
exempt from this check; keeping it a separate crate is what preserves the guarantee.

## Scenes are user input — never commit samples

Scene JSON is user-provided and may contain private or internal information. `scenes/`
is git-ignored on purpose. Do not commit sample scenes; use neutral, freshly-written
inline examples in docs. Before publishing anything, scan for content that should not
be public.

## Traps (look like cleanup, but will break things)

- **`roughr` 0.12 `ops_to_path` / `to_paths` is buggy**: it writes `L` (lineto) for
  `OpType::Move`, so the path has no initial moveto and `resvg` renders nothing.
  `backend.rs` builds the SVG `d` string itself from `Drawable.sets`
  (Move→`M`, LineTo→`L`, BCurveTo→`C`). Do **not** "simplify" it back to `to_paths`.
- **`dagre` is insertion-order sensitive.** The resolver sorts nodes by id and edges
  by `(from, to)` before feeding dagre so the JSON's authoring order does not reshuffle
  the layout. Keep that sort.
- **Parallel edges need a unique dagre edge name** (multigraph). Without it, two `a→b`
  edges collapse into one and a label is lost.
- **Human and agent must see the same pixels.** The live viewer shows the
  *server-rendered* PNG (via resvg), not a browser-rendered one, so the agent's
  self-check image and the human's view are identical. Do not move rendering to the
  browser.
- **Text is intentionally not hand-drawn.** Only shapes go through roughr; labels and
  notes are plain SVG `<text>` (same as Excalidraw). This is deliberate, not an
  oversight.
- **Determinism: same scene → same image.** Seeds derive from stable ids, not indices,
  so inserting a node does not re-jitter every other shape. Don't switch to index-based
  seeding.

## Architecture intent

Input is parsed into `InputScene` (geometry optional) and resolved into a strict,
render-ready `Scene`. Keep the strictness (`deny_unknown_fields`, validation) on the
`Scene` side and put all leniency / auto-layout in the input layer. A scene without a
`layout` block stays in manual-coordinate mode and must remain fully backward
compatible.

## Non-goals (deliberately omitted — more capability is not automatically better)

- Orthogonal / custom edge routing beyond what dagre produces.
- Force-directed or grid layouts.
- `.excalidraw` import/export — conversion drifts the layout and is not worth the upkeep.
- Inline text emphasis (bold / colored spans within a label) — left out to keep the
  surface small.

If you think one of these is worth adding, raise it explicitly rather than sliding it in.

## Conventions

- Code comments and commit messages in English.
- Before considering a change done: `cargo fmt`, `cargo clippy --all-targets
  --all-features -- -D warnings` (zero warnings), `cargo test`.

## Font

The bundled Yomogi font is SIL OFL 1.1 (`assets/fonts/Yomogi-OFL.txt`). Keep the
license file beside the font, and do not ship a modified font under the Yomogi name.
