# TPT Cut List Optimizer — Build Todo

> Hub listing: `tptsolutions.co.nz/tools/cutlist-optimizer` (registry slug
> `cutlist-optimizer`, category `construction`). Spec: `apps.md` §T1-7 in the
> tpt-electrician-nz-2 repo. Free WASM edition runs in-browser; the paid
> offline edition ($49 USD, Gumroad) adds multi-sheet/multi-material
> optimization, saved material libraries, and print-ready shop-floor layout
> export. UI built on `tpt-appfront` (DOM backend for the hub, webview for
> the desktop exe).
>
> Hub contract: `wasm-bindgen --target web` glue exposes `mount(container)`
> (called by `WasmAppRunner` after `default()` init). Standalone `trunk serve`
> mounts into the `#tpt-appfront-root` marker div instead — never `document.body` —
> so the two hosts never double-mount.
>
> **Status 2026-09-13:** Phases 0–2 complete and verified; Phase 3's
> multi-material solver + UI, stock-set library and print export are done
> (free bundle 75.7 KB gz, pro bundle 98.7 KB gz, desktop exe builds).
> Remaining items are blocked on the web repo / Gumroad (not on this machine),
> exe-side JSON persistence (needs a request/response IPC in appfront-webview),
> and the interactive desktop smoke test.

## Phase 0 — Scaffold & pipeline proof
- [x] `tpt-appfront init tpt-app-cutlist-optimizer --target dom` (path deps on the tpt-appfront checkout)
- [x] Restructure scaffold to hub contract: `#[wasm_bindgen] pub fn mount(el)` + `#[wasm_bindgen(start)]` no-op unless `#tpt-appfront-root` present
- [x] `index.html` carries the `#tpt-appfront-root` marker div
- [x] Build pipeline script (`build.ps1` for Windows + `build.sh` + `justfile` wrappers): `cargo build --release --target wasm32-unknown-unknown` → `wasm-bindgen --target web --out-name cutlist-optimizer` → copy glue+wasm into the web repo `public/apps/cutlist-optimizer/` when `TPT_WEB_REPO` is set (glue named `cutlist-optimizer.js`, wasm keeps its `_bg.wasm` name — the glue resolves it via `import.meta.url`), otherwise `dist/web/`. Also pins `wasm-bindgen = "=0.2.126"` to the installed CLI and disables trunk's wasm-opt (`data-wasm-opt="0"` — modern Rust emits bulk-memory opcodes trunk's cached Binaryen rejects)
- [x] Hello-world shape verified: glue exports named `mount(container)` + `default` init, matching the `WasmAppRunner` contract; standalone `trunk build --release`/`trunk serve` mounts via `#tpt-appfront-root`. (Loading through the live hub page `/tools/cutlist-optimizer` in `npm run dev` remains to be done on the machine with the web repo — the `TPT_WEB_REPO` deploy path is ready.)
- [x] git init + first commit

## Phase 1 — Solver crate (the moat)
Separate `solver/` crate in this repo, `no_std`-friendly pure Rust, zero UI deps —
shared verbatim by the WASM and desktop editions and unit-testable on the host.

- [x] Domain types: `Part` (length, qty, label, material), `StockSpec` (length, optional max bars, material, price), `Problem { parts, stocks, kerf, trim }` (one or more stocks — the free edition passes one), `Solution { bars, waste, cost, utilization }`
- [x] Multi-stock + materials (Pro): material isolation (a part never lands on foreign stock), per-stock `max_bars` caps (enforced during packing with a clear `OutOfStock`), price objective with smallest/largest-stock packing variants, cheaper-feasible-wins
- [x] Feasibility & accounting: kerf-aware used-length accounting (`parts + (n-1)*kerf + trim <= stock`), trim loss per bar, per-part placement records (`Cut { part, length, offset }`)
- [x] Baseline: best-fit-decreasing 1D packing (deterministic, kerf-aware)
- [x] Improvement pass (feature `improvement`): repeated FFD with seeded rotated/jittered orderings + Fisher-Yates shuffles, first-improvement empty-bar local search (deterministic rounds + optional `Instant` deadline, e.g. 200 ms budget in the Pro app)
- [x] Exact reference solver (feature `exact`): branch-and-bound minimum-bar search with bin symmetry pruning for small single-stock instances — benchmarks the heuristics (`exact <= FFD` fuzz-tested)
- [x] Unit tests: known-optimal instances ({3×600, 1×450} in 2400 → 1 bar); kerf correctness (two 1195 parts + kerf 5 fit 2400; kerf 11 forces a second bar); trim charged per bar; infeasible detection (part > stock − trim → clear `PartTooLong`; `max_bars` → `OutOfStock`); determinism (same input → byte-identical output, baseline and improvement)
- [x] Property tests (proptest): no overfilled bar ever; every requested part placed exactly once; utilization ∈ [0, 1] — plus deterministic LCG fuzz against the exact solver

## Phase 2 — Free WASM UI (tpt-appfront-dom)
- [x] Input panel (mounted once, no re-render focus loss): parts textarea (`length x qty [label]` per line; spaced/glued, `x`/`X`/`*` separators, `#` comments), stock length, kerf width — field values are read from the DOM on Optimize so the panel is never re-rendered
- [x] Parse + validate with inline error messages (bad line with line number, part > stock, qty ≤ 0, bad stock/kerf, free-tier 2000-line cap) — all issues reported at once
- [x] Optimize button → solver run (in the click handler, off the effect graph) → results `Signal`
- [x] Results section (re-rendered via `tpt_appfront_dom::render` = `create_effect` + `MountedRoot::render` into the `.cutlist-results` placeholder; invariant child shape so reconcile is always in-place): summary line (bars used, offcut total, utilization %), per-bar cut diagram (nested containers as proportional bars via generated `.cutlist-w-<tenths>` width classes + injected `<style>`), parts placement table (`data_grid`)
- [x] Free-tier limit enforced: single stock length input only, baseline packing only (improvement pass compiled out — the app `pro` feature maps to `tpt-cutlist-solver/improvement`); upsell copy + Pro link
- [x] Sample data button (cabinet job: sides, shelves, rails, stiles) + Clear button
- [x] Styling pass: self-contained injected stylesheet scoped to `.cutlist-app` (neutral light/dark via `prefers-color-scheme`, no external CSS deps; the DOM backend applies `class` verbatim and ignores custom attrs, so elements hook by class)

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [x] `pro` feature gates the improvement optimizer through `tpt-cutlist-solver/improvement` (200 ms budget); the free build compiles it out entirely
- [x] Multi-stock / multi-material optimization: `Problem` holds any number of `StockSpec`s; parts and stocks carry material ids (a part never mixes materials); per-stock `max_bars` caps; objective = minimum total stock price (defaults to stock length). Packer opens new bars on the smallest adequate stock, and the solver also packs a largest-first variant and keeps the cheaper one; the improvement pass dissolves whole bars across stock lengths (cost-decreasing, material-safe)
- [x] Pro UI: stock textarea (`2400 x 4 @ply` per line), `@material` tags on part lines, inline validation (no stock for material, part longer than every matching stock, bad line syntax), per-bar material in the diagram headers and a Material column in the placement table, total cost in the summary
- [x] Saved stock-set library: named sets (stocks + kerf) persisted to localStorage — survives restarts in the desktop WebView2 profile; Load/Delete per set, Save from the live fields
- [x] Print-ready layout export: "Print layout" button renders a print-only shop-floor sheet (per-bar diagrams + placement table + totals) via `window.print()` and a print stylesheet; works in the WebView2 shell
- [ ] Material library persisted to a JSON file **next to the exe** (currently profile storage; needs a request/response IPC addition to appfront-webview — app commands are fire-and-forget today)
- [x] Desktop shell via `tpt-appfront-webview` (wry/tao) hosting the same DOM bundle: `desktop/` bin (`tpt-cutlist-optimizer-pro`) serves `dist/` (dist resolution: `TPT_CUTLIST_DIST` → `./dist` → exe-adjacent); `build.ps1 -Pro` = pro wasm + `trunk build --release --features pro` + exe
- [x] Windows exe build verified: `target/release/tpt-cutlist-optimizer-pro.exe` compiles and exits with a clear message when `dist/index.html` is missing. (Interactive smoke test — open window, run sample job, check the Pro engine note, print — still to do in a desktop session)
- [ ] Gumroad: create product from apps.md listing copy ($49), upload exe, paste URL into `gumroadUrl` in the hub registry

## Phase 4 — Ship & measure
- [x] WASM payload size check (build.ps1 reports it every run): free **73.9 KB gz**, pro **77.8 KB gz** — far under the ~400 KB first-load target
- [ ] Registry entry `status: 'live'` with `wasm: { entry: '/apps/cutlist-optimizer/cutlist-optimizer.js' }` (web-repo side)
- [ ] Verify hub card, detail page, sitemap entry, and runner load in dev; verify prod build after deploy (web-repo side; set `TPT_WEB_REPO` and run `build.ps1` to stage the bundle)
- [ ] Cross-link from relevant construction calculators on fieldservice + llms.txt mention (web-repo side)
