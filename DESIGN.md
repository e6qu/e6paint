# e6paint — Design

*A canvas-first, multiplayer, deeply-zoomable whiteboard where code-drawn diagrams are first-class citizens.*
Rust core, compiled to WebAssembly, web-only at launch. Clean-sheet successor to
[mermollusc](https://github.com/e6qu/mermollusc) (reference copy: `reference/mermollusc`, read-only, not part of this repo).

Repo: `github.com/e6qu/e6paint` · License: **AGPL-3.0-or-later** · Status: design phase

---

## 1. Vision

One infinite canvas is the document. Freehand strokes, shapes, sticky notes, images, frames,
and **code-defined diagrams** (typed in a native DSL, auto-laid-out, expanded to live elements)
coexist as first-class scene objects. Boards are local-first CRDT documents that optionally go
multiplayer through a lightweight relay. Deep zoom (Prezi-style frames + present mode) makes a
board navigable at scales from a sticky note to a whole system map.

**What mermollusc DNA we keep:** the pure-core/imperative-shell discipline, fail-loud error
philosophy, layout-IR cleanliness, the icon/theme/export thinking, the one-branch engineering
contract. **What we discard:** text-as-source-of-truth, two-way source patching, the 15 ad-hoc
grammars, Canvas2D rendering, TypeScript.

## 2. Decision log (interview results, 2026-10-17)

| # | Decision | Choice | Rationale / notes |
|---|----------|--------|-------------------|
| D1 | Core paradigm | **Canvas-first, code inside** | One CRDT scene graph is the document; DSL text is an input that expands to elements. No invisible two-way sync — the complexity that rotted mermollusc does not exist here. |
| D2 | Zoom model | **Deep zoom** (Prezi-style frames, present mode, LOD) | "Large levels of zoom in/out" confirmed. Frames are narrative waypoints; LOD keeps huge sheets fluid. |
| D3 | Syntax stance | **Native DSL + Mermaid import** | A small clean grammar (hybrid arrows + blocks); Mermaid/DOT import via translators, pragmatic subset grown against fixtures (mermollusc's proven approach). |
| D4 | Runtimes | **Web-only** at launch | Pure static WASM app (GitHub Pages–hostable); the only server component is the collab relay. Native shells are explicitly out of scope for v1.0. |
| D5 | CRDT engine | **Loro** | Rust-native, rich containers (map, movable-list, tree, text) that map 1:1 onto a scene graph. First-class Rust API, WASM target. |
| D6 | DSL ↔ canvas | **Expand-to-elements** | Typing DSL produces live, individually-editable elements (one-way). Re-apply regenerates. Optional pinning of user-adjusted positions. |
| D7 | Performance target | **10k elements fluid** (60fps pan/zoom) | Calibrates LOD, culling, display-list caching. Not Miro-class 100k — that's a post-1.0 LOD-tier project. |
| D8 | Collab MVP | **Local-first + live rooms** | Boards live in OPFS/IndexedDB; "share" spins up a room: live presence, cursors, CRDT sync, periodic server snapshots. No accounts. |
| D9 | Renderer | **wgpu + Vello** | GPU vector rendering, zoom-independent quality. Text: cosmic-text/parley. Fallback path for non-WebGPU browsers is a designed milestone (see §8, R3). |
| D10 | UI chrome | **egui** (egui_wgpu) | Immediate mode, WASM-ready, coexists with Vello via render callbacks. Scene and chrome are separate render paths. |
| D11 | Auto-layout | **Pure-Rust layout engine** | No ELK/elkjs. Staged engines: layered (sugiyama) + tree/radial + lane/slab + grid; orthogonal router as its own milestone. Biggest engineering risk (R3). |
| D12 | Board file | **CRDT-native `.e6p` + exports** | Native = Loro snapshot, autosaved to OPFS. Exports: SVG, PNG, JSON dump. Git-versioning happens via exports, not the native file. |
| D13 | Launch families | **All 15 mermollusc families + DOT import** | Flowchart, sequence, C4, block, network, cloud, state, ER, class, requirement, gitGraph, timeline, mindmap, pie, gantt. Destination scope, sequenced in DSL waves (§11). |
| D14 | Board tools v1 | **All four bundles** | Core drawing kit (pen/shapes/connectors/text/sticky/image) + sketch theme + frames/present + library/layers/minimap. |
| D15 | License | **AGPL-3.0-or-later** | Parity with mermollusc. Icon packs: only AGPL-compatible licenses bundled; vendor cloud packs user-loaded, never redistributed. |
| D16 | Backend v1 | **Relay + snapshots** | Single self-hostable Rust binary (axum + SQLite): WS rooms, token room-auth, Loro snapshot persistence, history GC. App stays static. |
| D17 | Release strategy | **Big-bang v1.0** | No public release until the full scope lands. Internal milestones (§11) keep progress reviewable; risk acknowledged (R5). |
| D18 | DSL flavor | **Hybrid: arrows + blocks** | Mermaid-familiar edges (`a --> b`, `a -->\|yes\| b`) + D2-style blocks for nesting/containers/frames. |

## 3. Non-goals for v1.0

- Native desktop/mobile shells (the Rust core keeps them cheap to add later).
- Accounts, SSO, per-board permission tiers (tokens only; auth lives behind the relay seam).
- 100k+ element boards (10k target; LOD architecture leaves headroom).
- Any canvas↔text two-way patching. The DSL is an input/expansion mechanism, period.
- IE-grade browser support; we target evergreen Chrome/Edge/Firefox/Safari with a defined
  degradation ladder (§8).

## 4. System architecture

Cargo workspace. The core/shell purity rule is inherited from mermollusc's contract:
**core crates are pure** (no IO, no logging, no panic outside exhaustive-match asserts; errors
are `Result` values), **shell crates own all impurity** (GPU, DOM, network, storage, logging).

```
                         ┌──────────────────────────── shell ────────────────────────────┐
                         │  app (eframe/wasm)         relay (axum, native bin)           │
                         │  winit·wgpu·egui·Vello     rooms·tokens·snapshots             │
                         └───────▲──────────────────────────────▲───────────────────────┘
                                 │ display-list / events        │ WS: updates + presence
┌────────────────────────────────┴──────────────────────────────┴───────────────────────┐
│ editor   tools state machine · hit-test · transforms · present mode · clipboard       │
├───────────────────────────────────────────────────────────────────────────────────────┤
│ ui (egui chrome)      render (Vello display list, LOD, sketch)      collab (sync glue)│
├───────────────────────────────────────────────────────────────────────────────────────┤
│ dsl (native parser + mermaid/DOT import)   layout (layered·tree·lane·grid·router)     │
├───────────────────────────────────────────────────────────────────────────────────────┤
│ doc (Loro document mapping, undo)          scene (elements, geometry, spatial index)  │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

Dependency rule: arrows point downward only; no core crate imports a shell crate. Enforced by
review + a CI `cargo-modules`/`cargo-deny` check where practical.

### Crate map

| Crate | Kind | Owns |
|-------|------|------|
| `scene` | core | Element model (closed enum of kinds), geometry, typed IDs, z-order, spatial index (quadtree), LOD tier definitions, hit-test primitives |
| `doc` | core | The Loro mapping: containers ↔ scene operations, transactions, undo integration, snapshot import/export, JSON dump |
| `dsl` | core | Native DSL lexer/parser (spanned errors, recovery), printer, Mermaid→DSL translators (family by family), DOT importer |
| `layout` | core | Pure layout engines: layered (sugiyama), tree/radial, lane/slab (sequence, gitGraph, timeline), grid (block, gantt), chart (pie), orthogonal router; deterministic, seeded, property-tested |
| `render` | core | Scene → cached Vello display list: styles, sketch-mode stroke roughening (seeded, cache-only — never stored), text shaping cache hooks, SVG writer, LOD tier application, viewport culling |
| `editor` | core | Tools state machine (select/pen/shape/connector/text/sticky/image/frame/pan), selection model, align/distribute, group ops, present-mode sequencer, clipboard model |
| `collab` | core+shell-traits | Sync glue: Loro update batching/throttling, presence model, reconnect/backoff policy, room token handling (transport injected via trait) |
| `ui` | shell | egui panels: toolbar, outline, layers, inspector, library, dialogs, menus, theme (light/dark/sketch preview) |
| `app` | shell, wasm | eframe app: shared wgpu device, Vello pass + egui pass composition, OPFS persistence, worker offload (layout/import), web entry |
| `relay` | shell, native | axum server: WS rooms, token auth, SQLite (rooms/snapshots/GC), rate limits, single-binary deploy + Dockerfile |

Build/dev tooling: `trunk` for the web app, `wasm-pack`-compatible wasm32 target, `cargo` + a
root `Makefile` fan-out (`make check` = fmt + clippy -D warnings + test + trunk build, the gate)
mirroring mermollusc's uniform-targets discipline.

## 5. Data model (the Loro mapping)

One Loro `LoroDoc` per board. Containers:

| Container | Type | Holds |
|-----------|------|-------|
| `meta` | map | title, theme, created-at, settings |
| `elements` | map (id → sub-map) | per-element fields: `kind`, geometry (`x,y,w,h,rot`), style (`fill,stroke,stroke_width,sketch_seed`), text (for text elements), payload refs |
| `zorder` | movable list | element IDs in paint order — index = z |
| `tree` | tree | logical membership: frames ⊃ groups ⊃ elements (membership is logical, *not* spatial nesting) |
| `dsl_blocks` | map (id → text) | DSL source of diagram blocks; expansion output carries provenance `(block_id, node_key)` |

- **IDs:** client-minted typed IDs (`el_*`, `fr_*`, `blk_*`; prefixed ULID-style). Never reused.
- **Coordinates:** absolute board coordinates on every element. Frame/group moves propagate to
  members as explicit transactions (move-together), which keeps the renderer and hit-tester
  trivially simple. Revisit only if move-together churn proves costly under collab.
- **Freehand strokes:** quantized delta-compressed point arrays (`i32` lists); smoothing and
  roughening are render-time, derived, never stored.
- **Undo/redo:** Loro's undo manager scoped per participant; one gesture = one transaction.
- **Snapshots:** `doc.export(.snapshot())` → `.e6p`; relay and OPFS store snapshots plus tails of
  binary updates for cheap incremental sync.
- Text elements and DSL bodies are Loro `text` containers, so concurrent label edits merge.

## 6. Scene & rendering pipeline

Per frame: `doc → scene cache → culled, LOD-filtered display list → Vello pass` composed with
the egui pass on one shared wgpu surface.

- **Spatial index:** loose quadtree over element AABBs, maintained incrementally; queries are
  viewport-cull, hit-test, and marquee.
- **LOD tiers (D7):** t0 full fidelity; t1 (element < ~12px screen) simplified strokes, text
  dropped for rect placeholders; t2 (zoomed way out) frames/chunks render as labeled blocks with
  counts. Tier choice is per-element from the zoom transform.
- **Display-list caching:** display list fragments cached per frame-region with damage tracking;
  panning re-projects, editing invalidates only touched fragments.
- **Sketch theme:** seeded jitter (deterministic per element + zoom bucket) applied when building
  fragments, so it looks hand-drawn without polluting geometry or collaborating as churn.
- **Text:** cosmic-text/parley shaping, glyph cache; font collection bundled (OFL), subset for
  wasm size.
- **Export:** SVG writer consumes the same display-list primitives; PNG via offscreen wgpu
  render-to-texture. JSON dump = lossy pretty scene dump for grepping (D12).

## 7. The DSL ("mol") and importers

Hybrid arrows + blocks (D18). Sketch:

```
board "Payment redesign" { theme: sketch }

frame "Overview" { target: flow-auth }        // present-mode waypoint

flow "Auth flow" as flow-auth {
  start[Start] --> gate{Valid?}
  gate -->|yes| db[(User DB)]
  gate -->|no| err("Reject")
}

seq "Login" { alice ->> bob: "hello"; bob -->> alice: "hi" }

er "Shop" { CUSTOMER ||--o{ ORDER : places }

mind "Roadmap" { e6paint { collab { relay }  dsl { mermaid-import } } }
```

- Header keyword selects the family (the one mermollusc idea worth keeping); block braces give
  one consistent grammar instead of 15.
- `flow`/`state`/`er`/`class`/`req`/`block`/`cloud`/`network` share the arrow/statement core;
  `seq`/`git`/`timeline`/`gantt`/`pie`/`mind` have family-specific statements with shared lexing.
- Expansion (D6): parse → layout → stamp elements with provenance → insert as a locked-until-
 ungrouped set; **Re-apply** regenerates from source, honoring per-element `pin` flags.
- **Mermaid import:** family-by-family translators mermaid-text → `mol` (pragmatic subset,
  fixture-grown, located errors), then the same expansion path. **DOT import:** digraph/graph →
  `flow`. Import lives in `dsl` so the app only ever knows one pipeline.

## 8. Renderer path (D9/R3) — resolved (M8)

Primary and only paint path: **Vello on WebGPU** (evergreen 2026 browsers). The earlier
"degradation ladder" idea was resolved during M8 by eliminating it:

- eframe is compiled with its default feature set — `wgpu` on, `glow` off — so egui itself
  can only paint through wgpu. If adapter acquisition fails, eframe errors before the app
  is constructed; if `wgpu_render_state` is somehow absent anyway, `E6App::new` returns an
  error and the app refuses to start. Blitting any CPU-raster fallback texture would itself
  require a working wgpu queue, so an in-app software canvas is structurally unreachable —
  a fallback that cannot run is noise, and per the fail-loudly contract it does not exist.
- The resvg CPU rasterizer lives where it works: **offscreen PNG export** in `render`
  (`rasterize_view`), reusing the same SVG scene path as `export_svg`.
- WebGL2/`tiny-skia` (porting Vello fragments) was rejected: high effort, and it cannot
  rescue the only environment that needs rescue (no WebGPU at all).

## 9. Collaboration (D8/D16)

- **Transport:** WebSocket to relay. Two channels multiplexed: `sync` (binary Loro updates,
  snapshot exchange) and `presence` (ephemeral JSON: cursor, selection, name, color — never in
  the CRDT).
- **Rooms:** token-gated (opaque capability tokens, write; read-only later). No accounts (§3).
  Read-only shipped (M6): the relay marks the peer at join and rejects its batches; presence
  still flows.
- **Session modes (web):** a page opened via a share link is *room-first* — board content
  comes exclusively from the room; the local demo seed, OPFS restore, and autosave slot stay
  out of the way (a joiner's local board can never leak into someone else's room). Pages can
  point at any relay with `?relay=https://…` (validated http(s)); without it, the local dev
  default applies. Reset on a room-first session clears the board and syncs the clear.
- **Sync:** clients push throttled update batches (~30ms); server fans out and persists periodic
  snapshots (frontier + blob) to SQLite; late joiners get snapshot + tail. GC merges old tails.
- **Offline:** everything is local-first; edits queue as Loro updates and merge on reconnect
  (CRDT semantics, no conflict UI in v1; concurrent *moves* are last-writer-wins per field —
  documented, acceptable at 10k-element-board scale for now).
- **Self-host:** one binary + SQLite file; TLS terminates at the user's reverse proxy or the
  bundled rustls option; official demo relay deployed separately (Fly.io or similar).

## 10. Performance plan (D7)

Budget: 60fps pan/zoom at 10k elements, interactive (≤100ms) tool gestures on 1k-element
selections, board load ≤ 1.5s warm.

- Culling via the quadtree; LOD tiers as in §6; fragment caching; shaping cache.
- Layout and Mermaid/DOT import and expansion run on the main thread in v1.0
  (measured cheap: the 10k-board display rebuild is ~0.29 ms; imports are
  user-triggered and small). Moving them to a Web Worker is parked in §14 — the
  postMessage design needs no shared memory, so no COOP/COEP headers either way.
- WASM size budget: ≤ 5 MB gz total; measured in CI; fonts subset, allocator decision
  deferred to measurement (verify-in-spike). Walking-skeleton measurement (vello 0.11 +
  wgpu 30 + eframe 0.37, release + wasm-opt): **6.88 MB raw / 2.50 MB gz** — budget met
  with headroom.
- Benchmarks: `criterion` for layout/index/hot loops; a scripted "torture board" (10k mixed
  elements) replayed headlessly in CI smoke builds.

## 11. Milestones (internal ladder to the big-bang v1.0, D17)

| M | Name | Exit criteria (all internal; no public release before v1.0) |
|---|------|-------------------------------------------------------------|
| M0 | Walking skeleton | Workspace + CI; eframe/egui/Vello app runs in browser; pan/zoom viewport; `make check` gate green |
| M1 | Scene core | Loro doc mapping, elements, select/move/undo, OPFS autosave + restore, JSON dump |
| M2 | Whiteboard kit | ✅ Pen/shapes/connectors/text/sticky, image embeds, align/distribute, group/lock (flat tags; the nested tree lands with frames), z-order, sketch theme |
| M3 | Deep zoom | ✅ Frames + membership (attach-on-create/reattach-on-move), present mode with animated camera, minimap, library + layers panels, LOD v1 (skip <2px, placeholder <12px), torture-board bench ~0.38 ms/16 ms |
| M4 | DSL wave 1 ✅ + wave 2 ✅ + wave 3 ✅ | `mol` parser (spanned errors, recovery), canonical printer with round-trip, wave 1: flow/state/mind + Mermaid flowchart import; wave 2: seq (lane layout), er/class (member lists + `<|--` inheritance), provenance + re-apply (lock = pin); wave 3: git/timeline/pie/gantt chart blocks |
| M5 | Multiplayer ✅ | Relay binary, rooms + tokens, presence, snapshot persistence, offline queue + reconnect |
| M6 | Closeout ✅ | Read-only room tokens (view-only peers rejected at the relay), deep room-doc compaction (`COMPACT_EVERY_BATCHES`) |
| M7 | Exports + interchange ✅ | SVG export (vector, content-bounds viewBox), JSON export, PNG export (resvg raster), DOT import → mol flow, share links (room + token in the URL, clipboard copy, query-param join) |
| M8 | Hardening ✅ | Fallback question **resolved** (single Vello/WebGPU paint path; missing render state refuses to start — see §8), fail-loudly sweep (no silent `let _ =`/implicit defaults; chart rows error loudly), canvas a11y label (`WidgetInfo`/`Role::Canvas`), docs pass, security review (cargo-audit in CI, exit 0 — no vulnerabilities; transitive *unmaintained* warnings via loro→im/sized-chunks accepted upstream), perf budget proof (10k-board display rebuild ~0.29 ms of 16 ms; wasm 4.67 MB gz of 5 MB) → **v1.0** |

Waves exist to keep each milestone demonstrable in-repo; per mermollusc's §0.11, exactly one
branch/PR in flight at a time.

## 12. Engineering contract (mermollusc DNA, Rust-ified)

To be canonized in `AGENTS.md` at M0; summary:

1. **Fail loudly, no fallbacks** — errors are `Result`s returned to a boundary that surfaces
   them; no empty catches, no silent defaults. Retries with backoff for real network flakiness
   are allowed; they are not fallbacks.
2. **Pure core, impure shell** — core crates: no IO, no logging, no `unwrap`/`expect` outside
   exhaustive-match asserts; clippy pedantic subset denies in CI. Shell crates own GPU/DOM/net.
3. **Don't guess** — claims trace to source, spec, or a command. Unverified library behavior is
   a marked *verify-in-spike* item, never an assumption (this document has several, on purpose).
4. **Docs move with code** — `DESIGN.md` + per-crate README updated in the same change; `PLAN`
   status snapshots at milestone boundaries, not per-line rot.
5. **One branch, one PR** (hard rule, inherited verbatim in spirit from mermollusc §0.11).
6. **Supply chain** — new deps ≥24h old at pin time, `cargo audit` + `cargo vet` in CI,
   `Cargo.lock` committed, MSRV pinned; bundled assets (fonts, icons) carry source URL + license
   + pinned commit, AGPL-compatible only.
7. **Testing pyramid** — property tests (proptest) for DSL round-trip, layout determinism, scene
   ops; display-list goldens for render; Playwright e2e for the web shell; benches for hot paths.

## 13. Risk register

| ID | Risk | Mitigation |
|----|------|------------|
| R1 | Pure-Rust orthogonal routing underdelivers (ELK-grade ports/labels/clusters is months) | Router is its own milestone (M6) with a graded spec; wave-1/3 families don't depend on it; escape hatch preserved: a `LayoutProvider` trait keeps a JS/elkjs provider possible without call-site changes |
| R2 | Vello/wgpu WASM size or init cost busts the budget | Measured: 2.50 MB gz at the walking skeleton, 3.59 MB gz with the Loro document layer (budget 5 MB) — on track; monitor per milestone |
| R3 | Non-WebGPU browsers degrade badly | Fallback ladder designed as M8 deliverable, not an accident; LOD is the enabler |
| R4 | Loro API churn / missing container semantics | Scene↔doc mapping isolated in `doc` crate (thin seam); doc tests pin behavior; Loro snapshot format versioned in `.e6p` header |
| R5 | Big-bang release starves feedback | Internal demo boards + torture-board CI keep quality visible; milestones are reviewable in-repo even without public tags |
| R6 | 15-family scope creep re-inflates the mermollusc problem | One shared statement/arrow core in `dsl` (D18) — families are thin ASTs over one grammar, one layout contract |

## 14. Open questions (parked, decide at the marked milestone)

- DSL name/branding beyond "mol" (informal) — decide at M4. *(informal "mol" stuck; revisit
  only if it ever ships as a standalone tool)*
- Element coordinate model: absolute-only (current) vs frame-relative transforms — revisit if
  collab move-churn shows up (M3). *(M3 shipped absolute-only; no churn observed)*
- Read-only room tokens & board listing UX — M5. **Resolved (M6):** read-only tokens shipped;
  board listing stays out of scope (share links are the discovery path).
- Share-link encoding size cap vs OPFS-based sharing — M7. **Resolved (M7):** links carry
  room id + token in the query string; boards themselves stay in the room's server snapshot.
- Official demo relay hosting (Fly.io vs VPS) — parked; `cargo run -p e6-relay` and
  `E6PAINT_RELAY_ADDR` are the supported story for v1.0.
- Accessibility scope for v1.0 — **Resolved (M8):** keyboard completeness (tool shortcuts,
  undo/redo, Esc/Enter editing) plus a labeled canvas node (`WidgetInfo`/`Role::Canvas`);
  full screen-reader coverage of canvas content is post-v1.0.
- Web Worker offload for layout/import/expansion (§10) — did not ship in v1.0
  (main thread measured cheap); park until import latency shows up in practice.
