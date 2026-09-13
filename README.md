# tpt-app-cutlist-optimizer

TPT Cut List Optimizer — kerf-aware 1D cutting-stock optimization for
woodworkers. Two editions ship from this one repo:

- **Free WASM edition** — runs in-browser on
  `tptsolutions.co.nz/tools/cutlist-optimizer` (registry slug
  `cutlist-optimizer`, category `construction`). Single stock length,
  deterministic best-fit-decreasing packing.
- **Pro desktop edition** ($49, Gumroad) — the same UI built with
  `--features pro`, which compiles the solver's improvement search in, hosted
  in a native webview shell (`desktop/`) as a Windows exe.

UI is built on [`tpt-appfront`](https://github.com/tpt-solutions/tpt-appfront)
(DOM backend), solver logic lives in the UI-free `solver/` crate shared
verbatim by both editions.

## Layout

```
solver/            pure Rust 1D cut list solver (the moat, host-testable)
  src/model.rs     Part, StockSpec, Problem, Cut, Bar, Solution + accounting
  src/error.rs     SolveError (validation, part-too-long, out-of-stock)
  src/ffd.rs       deterministic best-fit-decreasing baseline
  src/improve.rs   improvement pass (feature `improvement`): randomized
                   restarts + empty-bar local search, time-boxable
  src/exact.rs     exact branch-and-bound minimum-bar solver (feature `exact`)
  tests/           known-optimal, kerf, infeasibility, determinism, proptest
src/
  lib.rs           wasm-bindgen hub contract: `mount(el)` + `start()`
  app.rs           the DOM app (input panel mounted once, reactive results)
  parse.rs         `length x qty [label]` textarea parser + validation
desktop/           tpt-appfront-webview shell hosting the trunk bundle
build.ps1          wasm pipeline: cargo -> wasm-bindgen -> hub bundle
build.sh           same for macOS/Linux
justfile           thin task wrappers (build/pro/test/check/dev)
index.html         standalone dev page (#tpt-appfront-root marker div)
```

## Hub contract

`wasm-bindgen --target web` glue exposes the named export
`mount(container)` plus the `default` init export. The hub's
`WasmAppRunner` dynamic-imports the glue, awaits `default()`, then calls
`mount(container)`. The glue file is deployed as
`public/apps/cutlist-optimizer/cutlist-optimizer.js` with the wasm beside it
as `cutlist-optimizer_bg.wasm` (resolved via `import.meta.url`). Standalone
`trunk serve` mounts into the `#tpt-appfront-root` marker div instead — never
`document.body` — so the two hosts never double-mount.

## Commands

```pwsh
just build        # free wasm bundle -> dist\web (+ gzip size report)
just pro          # pro wasm + trunk bundle + desktop exe
just test         # solver (default + all features) + parser tests
just check        # type-check wasm (free + pro) and the desktop shell
just dev          # trunk serve (standalone page)
```

Set `TPT_WEB_REPO` to the tptsolutions.co.nz web repo path and `build.ps1`
deploys the bundle to `<repo>\public\apps\cutlist-optimizer\` directly
(the glue is generated with the right name via `wasm-bindgen --out-name`).

## Solver features

| feature        | default | enables                                            |
| -------------- | ------- | -------------------------------------------------- |
| `std`          | yes     | wall-clock deadline in `ImproveOptions`            |
| `improvement`  | no      | randomized restarts + local search (Pro)           |
| `exact`        | no      | branch-and-bound optimal solver for small jobs     |

The app crate's `pro` feature maps to `tpt-cutlist-solver/improvement` plus
serde (the stock-set library); the free WASM bundle is built with default
features, so the improvement pass is compiled out entirely.

## Free vs Pro

| capability                        | Free        | Pro                              |
| --------------------------------- | ----------- | -------------------------------- |
| kerf/trim-aware 1D packing        | yes         | yes                              |
| packing engine                    | BFD (both stock-order variants) | BFD + improvement search (200 ms budget) |
| stock lengths per job             | 1           | unlimited, per-material          |
| materials (`@tag` on part lines)  | —           | yes (materials never mix)        |
| per-stock availability (`x n`)    | —           | yes (`max_bars` caps)            |
| cost objective (per-bar price)    | implicit (length) | yes, shown in the summary  |
| saved stock sets                  | —           | yes (localStorage/WebView2)      |
| print-ready shop-floor layout     | —           | yes (`window.print()` sheet)     |

## Pro input format

Parts (one per line): `length x qty [label] [@material]`, e.g.
`570 x 4 Side panel @ply`. Stocks (Pro, one per line):
`length [x max-bars] [@material]`, e.g. `2400 x 4 @ply`. `#` comments are
allowed in both.

## Solver guarantees (property-tested)

- no bar is ever overfilled: `parts + (n-1)*kerf + trim <= stock`,
- every requested piece is placed exactly once with its cut position,
- same input in, same bars out (the improvement pass is seeded from the
  problem itself; wall-clock deadlines are optional and off in tests).

