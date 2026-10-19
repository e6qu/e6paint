# e6paint

*A canvas-first, multiplayer, deeply-zoomable whiteboard where code-drawn diagrams are
first-class citizens.* Rust core, compiled to WebAssembly. Clean-sheet successor to
[mermollusc](https://github.com/e6qu/mermollusc).

**Architecture & decisions:** [DESIGN.md](DESIGN.md) · **Operating contract:** [AGENTS.md](AGENTS.md)
· **License:** AGPL-3.0-or-later

## Status — M0 ✅ · M1 scene core ✅ · M2 whiteboard kit ✅ · M3 deep zoom ✅ · M4 DSL waves 1–3 ✅ · M5 multiplayer ✅ · M6 closeout ✅ · M7 exports + share links ✅ · M8 hardening ✅ → **v1.0**

A pan/zoom infinite canvas through **eframe + egui + Vello**, building for native and
**web (WebGPU/WASM via trunk)**. The board is a **Loro CRDT document**. Draw with the
tool bar: pen, rectangles, ellipses, arrow connectors, sticky notes, text (inline
editor), **frames** (elements created inside attach; dragging a frame carries its
members; dropping outside detaches), and PNG/JPEG images. Marquee multi-select with
shift-click; align/distribute; group/lock; z-order; sketch theme. **Present mode** walks
the frames with an animated camera (→/←/Esc). **Library** presets, **layers**, and a
**mol code panel** (flow/state/seq/er/class/git/timeline/pie/gantt diagrams from
text, Mermaid + DOT import, re-apply with provenance) sit on the
right; a **minimap** in the corner; LOD tiers keep far-out zoom cheap. **Export SVG**
produces a vector snapshot of the board alongside the JSON dump. cmd/ctrl+Z undo — every gesture is one step. Autosaved locally
(OPFS / cache dir), restored on load; Export JSON and Export SVG/PNG. Fresh installs
seed a demo board.

**Multiplayer**: Share copies a join link — room id + access token in the URL — to the
clipboard; the action bar has **Share edit** (write token) and, for the room's creator,
**Share view** (read-only token). Opening a link joins the room directly (a room-first
session: board content comes from the room, never from local demo/restore/autosave).
Hosted deployments point the client at their relay with `?relay=https://…`. Remote
cursors render live and edits sync as CRDT batches (full-history, idempotent) through
the relay; snapshots persist server-side and greet joiners. View-only tokens join
without edit rights; the relay compacts room docs periodically and auto-reconnects on
drops.

| | measured |
|---|---|
| wasm binary | 12.8 MB raw / **4.67 MB gz** with resvg PNG export linked (release + wasm-opt; budget: 5 MB gz) |
| gate | `make check` green: rustfmt, clippy `-D warnings` (host + wasm target), 104 tests, 2 browser e2e specs |
| realtime | relay e2e: join snapshots, batch fan-out, presence routing, token auth, read-only peers |
| renderer | single paint path: Vello on WebGPU; without a GPU surface the app refuses to start (fail loudly — CPU rasterization exists for PNG export, not as a canvas fallback) |
| display-list rebuild | **~0.29 ms** for the 10k torture board (`cargo bench -p e6-render`; 16 ms frame budget) |
| doc store | loro 1.16 (snapshot + JSON dump + undo + peer sync verified against source) |

Milestone ladder complete through M8 — **v1.0** ([DESIGN.md §11](DESIGN.md)). The
post-v1.0 backlog is empty; candidates for a future cycle: full screen-reader
coverage of canvas content, plugin-style DSL block extensions.

## Quick start

Prerequisites: [rustup](https://rustup.rs) (the pinned toolchain installs itself from
`rust-toolchain.toml`), then `make install-tools` once (adds the wasm32 target + trunk).

```sh
make run-web        # http://localhost:8080 (trunk serve)
make run-native     # native window
make check          # the gate: fmt-check + clippy -D warnings + tests
make build-web      # release web build into crates/app/dist
npm run pw:install  # once: Chromium for e2e (downloads into ./.playwright-browsers)
npm run test:e2e    # Playwright e2e against the live wasm app (boots trunk serve)
```

**Controls:** tool bar (Select · Pan · Pen · Rect · Ellipse · Arrow · Text · Sticky ·
Frame; digits **1–9** pick tools in that order) ·
wheel = pan · pinch or cmd/ctrl+wheel = zoom at pointer · drag empty canvas = marquee ·
shift-click = add/remove from selection · drag selection = move (groups move together) ·
Delete = remove · Esc = cancel · cmd/ctrl+Z undo (+shift/Y redo). The action bar has
Align/Distribute, Group/Ungroup, Lock/Unlock, Sketch toggle, Front/Back, Undo/Redo,
Reset, and Export JSON.

## Workspace

Ten-crate DAG, pure core / impure shell ([AGENTS.md](AGENTS.md) §Workspace DAG):

| crate | role |
|-------|------|
| `scene` | elements, geometry, camera, spatial index (pure) |
| `doc` | Loro CRDT board document (meta/elements/zorder, snapshots, undo) |
| `dsl` | native `mol` DSL: flow/state/mind/seq/er/class/git/timeline/pie/gantt + Mermaid/DOT import |
| `layout` | pure-Rust layout engines behind `LayoutProvider` (dagre-style flow, lanes, charts) |
| `render` | scene → Vello display list, LOD, SVG export (pure) |
| `editor` | tools, selection, gestures |
| `collab` | presence, throttling, transport port, share-query parsing |
| `ui` | egui chrome: HUD, panels |
| `app` | eframe shell: native + web |
| `relay` | axum collab server: rooms, write/read tokens, snapshots, compaction — `E6PAINT_RELAY_ADDR`, `E6PAINT_RELAY_DB`, `E6PAINT_COMPACT_EVERY` env |

`reference/mermollusc` is a read-only reference clone, excluded from the repo.

## License

AGPL-3.0-or-later, verbatim [LICENSE](LICENSE). Bundled assets will carry source URL,
license, and pinned commit; vendor icon sets stay user-loaded, never redistributed.

Copyright (c) 2026 Adrian Mârza and e6paint contributors
