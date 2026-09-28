# Gumroad listing

Everything needed to create/update the Gumroad product. Source copy is
`apps.md` §T1-7 in the `tpt-electrician-nz-2` repo — if that changes, update
here too (and vice versa).

## Step-by-step: publish this listing

1. Build the Pro edition: `./build.ps1 -Pro` (see `todo.md`/`README.md` for
   details) — produces `release\TPT-Cut-List-Optimizer-Pro.zip`.
2. Log in to Gumroad → **New product** → **Digital product**.
3. Set the product name, price, and upload the packaged file (all below).
4. Paste the short description, bullets, and full description (below) into
   their respective fields.
5. Set category/tags (below) and add a cover image (see note below).
6. Publish, then copy the resulting product URL (`https://gum.co/...`).
7. Paste that URL into the `cutlist-optimizer` entry's `gumroadUrl` field in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$99'`
   on the same entry.
8. Confirm `status: 'live'` on that entry (it already is — the free demo
   shipped first).
9. Commit and push the registry repo.

## File to upload

```
release\TPT-Cut-List-Optimizer-Pro.zip
```

Already built as of this writing — re-run `./build.ps1 -Pro` and re-zip
before every price/content change to the Pro build.

## Product name

```
TPT Cut List Optimizer — Minimize Material Waste with Real Optimization
```

## Price

**$99 USD, one-time purchase** (no subscription).

## Short description / summary field

```
Better material utilization than basic packing tools, built on a real optimization solver. For cabinet makers, metal fabricators, and glass shops who waste real money on offcuts.
```

## Bullets / feature list

```
Real optimization, not greedy packing
Multi-sheet/multi-material
Print-ready shop-floor layouts
Saved material libraries
One-time purchase
```

## Full description (long-form field)

```
TPT Cut List Optimizer takes a list of required cut pieces and stock sheet/
bar sizes and computes an optimized cutting layout that minimizes waste —
using a real optimization solver, not the basic first-fit-decreasing
packing most free tools rely on.

Who it's for: cabinet makers, metal fabricators, and glass shops who waste
real money on suboptimal cutting layouts. CutList Plus and OptiCut prove
people already pay for this category — this is the same idea with a
genuinely stronger solver behind it.

Pro edition (this download):
- Multi-sheet and multi-material optimization in one job
- Saved stock-set libraries (named sets persisted between sessions)
- Offcut bank — save leftover material as reusable stock for next time
- Print-ready shop-floor cutting layouts
- Total cost rollup, not just piece counts
- CSV import/export and shareable job links

Try the free single-sheet edition first, right in your browser, no install:
tptsolutions.co.nz/tools/cutlist-optimizer

Requirements: Windows 10 (20H2+) or Windows 11. Uses the WebView2 runtime,
preinstalled on both — nothing extra to download.
```

## Category / tags

- Category: **Software > Utility** (or Gumroad's closest equivalent)
- Tags: `cutting-optimization`, `cabinet-making`, `metal-fabrication`,
  `woodworking`, `nesting`, `windows`

## Cover image / thumbnail

Not created yet. A screenshot of the per-bar cut diagram view with a real
multi-material job loaded (use the sample cabinet job button) is the most
convincing cover. No stock/AI art.

## After publishing

1. Copy the resulting product URL (`https://gum.co/...`).
2. Paste it into the `gumroadUrl` field of the `cutlist-optimizer` entry in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$99'`.
3. Confirm `status` is `'live'`.
4. Commit and push the registry repo.
