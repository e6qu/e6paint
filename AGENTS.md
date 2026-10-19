# AGENTS.md — e6paint operating contract

`DESIGN.md` is the architectural source of truth. This file is how we work inside it.
These rules **override convenience**.

---

## Hard rules

1. **Fail loudly, no fallbacks.** Errors are `Result`/`Option` values returned to a boundary
   that surfaces them. No empty catches, no silent defaults, no swallowed errors. Retries with
   backoff for genuine network flakiness are allowed; they are not fallbacks.
2. **Pure core, impure shell.** Core crates — `scene`, `doc`, `dsl`, `layout`, `render`,
   `editor`, `collab` — do no IO, no logging, no threads, and never call `panic!`, `unwrap`,
   or `expect` (workspace lints deny these). Shell crates — `ui`, `app`, `relay` — own
   GPU/DOM/network/storage and surface every error loudly.
3. **Don't guess.** Claims trace to source code, an official spec, official docs, or command
   output. Unverified library behavior is a marked *verify-in-spike* item (DESIGN §8, §14),
   never an assumption.
4. **Supply chain.** New dependencies are stable releases ≥24h old at pin time; versions come
   from `[workspace.dependencies]` only; `Cargo.lock` is committed. `cargo audit`/`cargo vet`
   join CI with the first dependency-heavy milestone. Bundled assets (fonts, icons) carry
   source URL + license + pinned commit; AGPL-compatible licenses only, vendor icon sets stay
   user-loaded, never redistributed.
5. **Docs move with code.** Crate docs and (when architecture shifts) `DESIGN.md` update in
   the *same* change. No "docs follow-up" commits.
6. **One branch, one PR. No exceptions.** Exactly one working branch and one open PR at a
   time; merge (or close) before starting the next. No stacking, no "getting ahead".
7. **No noise comments.** Comments explain a boundary, an invariant, or a non-obvious "why" —
   nothing else. No milestone/phase numbers in source or comments; the why goes in the commit
   message.
8. **Tests live under the same lints.** No `unwrap`/`expect` in tests; test fns return
   `Result` and use `?` (see `relay`'s router test for the pattern).

## Workspace DAG

Arrows point at what a crate may import; importing upward is forbidden.

```
scene  <-  { doc, dsl, layout, editor, collab }  <-  render  <-  { ui }  <-  app
                 (all of these may also import scene directly; ui also imports editor)
relay  —  shares no code with the app graph; couples only over the wire protocol (DESIGN §9)
```

## Uniform make targets

`fmt` · `fmt-check` · `lint` (clippy, `-D warnings`) · `test` · `build-native` ·
`build-web` (trunk) · `run-native` · `run-web` (trunk serve) · `check` (= fmt-check + lint +
test — the gate) · `clean` · `install-tools`.

## Testing pyramid

- **Unit** (most numerous): pure crate functions, table-driven; property tests join with the
  dsl/layout milestones.
- **Integration**: shell + core against fixtures; `relay`'s router test is the pattern.
- **Goldens + e2e**: display-list goldens and Playwright flows join with the render/app
  milestones.
- **Perf**: the torture board (10k elements, DESIGN §10) is a first-class test input.
