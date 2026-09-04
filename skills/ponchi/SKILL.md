---
name: ponchi
description: >-
  Draw a hand-drawn-style diagram to align with a human on structure — state
  machines, flows, request paths, component maps, lifecycles. Use when a textual
  explanation would be easier to judge as a rough sketch, when the user asks for
  a diagram / 図 / ぽんち絵, or when you want a human to see and approve the shape
  of a design before you commit to it. Renders entirely on the local machine
  (no network), with a live browser view the human can watch.
---

# ponchi

ponchi turns a declarative scene (nodes + edges) into a hand-drawn-style diagram.
Everything runs locally — nothing is sent over the network. The point is **shared
understanding**: you sketch, the human watches the same picture live in a browser,
and you refine it by reading the rendered PNG yourself.

## The core loop (serve mode — the default)

Prefer `serve`. It gives the human a live view and lets you self-check the same
pixels they see.

1. **Make sure `ponchi` is installed** (see Setup).
2. **Start the server in the background from a session-specific temp work dir**,
   capture its PID, and wait until it is actually listening — poll the log, do
   not fixed-sleep (the log is often still empty right after launch):
   ```sh
   PONCHI_WORK_DIR="$(mktemp -d /tmp/ponchi-work.XXXXXX)"
   cd "$PONCHI_WORK_DIR"
   ponchi serve > serve.log 2>&1 &
   PONCHI_PID=$!
   for _ in $(seq 1 30); do grep -q listening serve.log && break; sleep 0.3; done
   cat serve.log
   # ponchi serve listening on http://127.0.0.1:<PORT>/
   # POST token (Authorization: Bearer ...): <TOKEN>
   printf 'Work dir: %s\nServer PID: %s\n' "$PONCHI_WORK_DIR" "$PONCHI_PID"
   ```
   The port and work dir are unique to this session, so concurrent sessions do
   not share the server or its files. Parse `<PORT>` and `<TOKEN>` from the log.
   The server writes `out/latest.{png,svg}` relative to `$PONCHI_WORK_DIR`, keeping
   the output out of any repo. Record the printed `<WORK_DIR>` and `<PID>` as well;
   shell variables may not survive into a later tool call, so substitute the
   recorded values in the remaining steps.

   If your environment kills background processes between commands (some sandboxes
   do — a later `curl` then fails and the PID is gone), don't fight it: use render
   mode instead (below). serve needs a process that survives across your commands.
3. **Tell the human the URL** so they can open it in a browser. The view
   auto-refreshes whenever you push a new scene.
4. **Write a scene** to the work dir (see Scene format) and POST it. Give the file
   a short, task-specific name instead of reusing `scene.json`; scenes are
   throwaway input and must stay outside the repo:
   ```sh
   PONCHI_WORK_DIR="<WORK_DIR>"
   PONCHI_SCENE="$PONCHI_WORK_DIR/request-lifecycle.json"
   # Write the scene JSON to "$PONCHI_SCENE", then:
   curl -s -X POST "http://127.0.0.1:<PORT>/api/scene" \
     -H "Authorization: Bearer <TOKEN>" \
     -H "Content-Type: application/json" \
     --data-binary "@${PONCHI_SCENE}"
   # -> {"version":N}   (HTTP 400 with a message if the scene is invalid)
   ```
5. **Self-check**: read `<WORK_DIR>/out/latest.png` and actually look at it.
   Confirm the POST response `version` went up and the image matches what you
   sent — if the version rises but the picture is stale, the server isn't running.
   Anything overlapping, mislabeled, or unclear? Fix the JSON and POST again. This
   is the whole value — don't skip it.
6. When done, verify the recorded PID still belongs to this server before stopping
   it. Don't use `pkill` or `kill %1`: the former can stop other sessions, and job
   control may not carry across commands.
   ```sh
   PONCHI_PID="<PID>"
   case "$(ps -p "$PONCHI_PID" -o command=)" in
     *"ponchi serve"*) kill "$PONCHI_PID" ;;
     *) printf 'Refusing to stop PID %s: not ponchi serve\n' "$PONCHI_PID" >&2; exit 1 ;;
   esac
   ```

`render` mode is the alternative: one-shot, no server, no human view. Use it for
CI or when you only need a file:
```sh
PONCHI_WORK_DIR="$(mktemp -d /tmp/ponchi-work.XXXXXX)"
printf 'Work dir: %s\n' "$PONCHI_WORK_DIR"
# In a later call, write <WORK_DIR>/request-lifecycle.json, then:
ponchi render "<WORK_DIR>/request-lifecycle.json" \
  -o "<WORK_DIR>/request-lifecycle.png"   # or use an .svg output
```

## Share on GitHub (only when asked)

ponchi itself stays local-only. Uploading its output crosses that boundary, so do
this only when the human explicitly asks to share the diagram. Immediately before
uploading, self-check the PNG again for private or internal content.

GitHub CLI 2.99.0 or later can attach the local PNG directly to an issue, pull
request, or comment on GitHub.com or GitHub Enterprise Cloud when you have push
access to the repository. Substitute the self-checked PNG path for `<OUTPUT_PNG>`:

```sh
gh --version
gh issue comment 123 --attach '<OUTPUT_PNG>#Request lifecycle diagram'
# Or, for a pull request:
gh pr comment 456 --attach '<OUTPUT_PNG>#Request lifecycle diagram'
```

If `gh --version` reports a version older than 2.99.0, stop and tell the human an
upgrade is required instead of trying another upload mechanism.

The same repeatable `--attach` flag works with `gh issue` and `gh pr` `create`,
`edit`, and `comment`. If the body already references the attached local path,
`gh` rewrites that reference to the uploaded URL; otherwise it appends the image.
See the [GitHub CLI attachment documentation](https://gh.io/gh-attach).

## Cleanup

After the self-check and any requested sharing are complete, remove only the
session-specific work dir created above. Validate the recorded path before deleting
it so a missing or incorrect value cannot broaden the cleanup target:

```sh
PONCHI_WORK_DIR="<WORK_DIR>"
case "$PONCHI_WORK_DIR" in
  /tmp/ponchi-work.??????) rm -r -- "$PONCHI_WORK_DIR" ;;
  *) printf 'Refusing to remove unexpected path: %s\n' "$PONCHI_WORK_DIR" >&2; exit 1 ;;
esac
```

## Setup

Check first; install only if missing:
```sh
command -v ponchi
```

If absent, install by **either** route (pick what fits the environment):

- **Download a release binary** (no Rust toolchain needed). From
  <https://github.com/cappyzawa/ponchi/releases/latest>, pick the asset for the
  OS/arch, verify it against `SHA256SUMS`, `chmod +x`, and put it on `PATH`.
  Assets: `ponchi-x86_64-unknown-linux-gnu`, `ponchi-aarch64-apple-darwin`,
  `ponchi-x86_64-apple-darwin`.
- **Build from source** (needs a Rust toolchain):
  ```sh
  cargo install --git https://github.com/cappyzawa/ponchi
  ```

The handwriting font is embedded in the binary, so it is self-contained — no
font files need to travel with it.

## Scene format

JSON. The recommended mode is **auto layout**: you describe *relationships only*
and the engine places everything. Do not write coordinates in auto mode.

```jsonc
{
  "schema_version": "ponchi.scene/v0",
  "layout": {
    "mode": "auto",
    "engine": "layered",
    "direction": "LR",     // "LR" left-to-right, or "TB" top-to-bottom
    "fit": "grow"          // grow the canvas to fit the content
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

- `shape`: `rect` | `ellipse` | `diamond`.
- `role` (optional, drives color): `input` | `process` | `external` |
  `persistence` | `transformation`. The names are just color buckets — choose by
  what reads naturally (e.g. an external client → `external`, a datastore →
  `persistence`, a worker → `process`), and keep the scheme consistent so color
  carries meaning. For a state machine specifically, a success/terminal state →
  `persistence` and an error state → `external` reads well; keep success and
  failure visibly distinct.
- `label`: any text; `\n` makes multiple lines. CJK works (the font renders it).
- `edge`: `from`/`to` reference node ids; `label` is optional; `style` is
  `arrow` (default) or `line`; `dashed: true` for dashed.
- `id`s must be unique within their kind; every edge endpoint must be an existing
  node id (otherwise the scene is rejected — a clear error, not a silent drop).

Optional annotations (these need coordinates even in auto mode): `texts`
(free-standing notes) and `zones` (labeled background boxes). With `fit: "grow"`
the canvas expands to include them. Keep them sparse.

Manual mode (omit the `layout` block) requires `x`/`y`/`w`/`h` on every node —
only reach for it when you need precise placement. Auto is almost always better.

## Traps (learn these before drawing)

- **Scenes are throwaway intermediate input — write them under `/tmp`, not in a
  repo.** The human cares about the diagram, not the JSON. Keeping scenes out of
  the working tree also means there is nothing to accidentally commit.
- **Drawing private/internal content is fine** — ponchi is local-only and sends
  nothing over the network, so sketch sensitive designs freely. Only when you
  *deliberately* share a rendered PNG/SVG should you review what's in it, like
  any other artifact.
- **In auto mode, don't set `x`/`y` on nodes** — it's rejected. Let the engine place.
- **Start `serve` in the background**, not the foreground — it runs until killed
  and will otherwise block you.
- **Always read back the rendered PNG** and judge it. The layout you imagined and
  the layout the engine produced are not the same; looking is how you catch it.
- **Keep labels short.** Long labels make wide nodes and crowd the layout; put
  detail in prose, keep the sketch structural.
- **If back-edges or cycles crowd the layout, try the other `direction`.** A
  returning edge (e.g. a retry loop) often routes more cleanly with `TB` than
  `LR` (or vice versa). Re-render and compare — it's a one-word change.

## When NOT to use ponchi

For a trivial 2–3 box idea, ASCII or prose is faster. ponchi earns its keep when
the structure (branches, cycles, multiple paths, lifecycles) is what the human
needs to see and judge.
