//! Free WASM edition UI (tpt-appfront-dom).
//!
//! Rendering architecture, per the build todo:
//! - the **input panel is mounted once** (`tpt_appfront_dom::mount`) — its
//!   nodes are never reconciled, so typing never loses focus; field values
//!   are read straight from the DOM when Optimize is pressed;
//! - the **results section re-renders fine-grained** via
//!   `tpt_appfront_dom::render` (a `create_effect` + `MountedRoot::render`
//!   pair) driven by the `results` Signal. Its tree keeps a stable child
//!   shape (container/text/data_grid kinds at fixed positions) so in-place
//!   reconciliation never has to full-replace.
//!
//! The DOM backend applies `class` verbatim but ignores custom attributes,
//! so elements are hooked by unique class names, proportional bar widths use
//! generated `.cutlist-w-<tenths>` classes backed by an injected `<style>`,
//! and the whole stylesheet is injected once at mount — no external CSS.

use std::collections::BTreeSet;
use std::rc::Rc;

use tpt_appfront_core::{Signal, UITree};
use tpt_cutlist_solver::{Part, Problem, Solution, StockSpec};
use wasm_bindgen::JsCast;

use crate::parse::{self, Issue, ParsedPart};

#[cfg(feature = "pro")]
use crate::library::SavedSet;
#[cfg(feature = "pro")]
use std::time::{Duration, Instant};

/// The Pro build runs the improvement search with a hard wall-clock budget.
#[cfg(feature = "pro")]
const IMPROVE_BUDGET: Duration = Duration::from_millis(200);

#[cfg(feature = "pro")]
const ENGINE_NOTE: &str =
    "Optimization: best-fit-decreasing + randomized improvement search, multi-material (Pro edition).";

#[cfg(not(feature = "pro"))]
const ENGINE_NOTE: &str =
    "Optimization: best-fit-decreasing (free edition — the paid Pro build adds an improved search, multiple materials and print export).";

#[cfg(not(feature = "pro"))]
const SAMPLE_PARTS: &str = "730 x 4 Cabinet side\n560 x 6 Shelf\n396 x 4 Top rail\n197 x 8 Door stile\n640 x 2 Door panel";
#[cfg(not(feature = "pro"))]
const SAMPLE_STOCK: &str = "2400";
#[cfg(feature = "pro")]
const SAMPLE_PARTS: &str = "730 x 4 Cabinet side @bench\n560 x 6 Shelf @bench\n396 x 4 Top rail @bench\n197 x 8 Door stile @bench\n640 x 2 Door panel @bench";
#[cfg(feature = "pro")]
const SAMPLE_STOCKS: &str = "2400 x 4 @bench\n3000 x 2 @bench";
const SAMPLE_KERF: &str = "3";

/// What the results section is currently showing.
#[derive(Clone)]
enum Outcome {
    Idle,
    Invalid(Vec<Issue>),
    Solved {
        solution: Solution,
        parts: Vec<ParsedPart>,
        /// The stock specs the job was packed against (per-bar lookup).
        stocks: Vec<StockSpec>,
        /// Material id -> display name (index 0 is always the unnamed
        /// default material).
        material_names: Vec<String>,
        kerf: u32,
    },
}

/// App events (buttons only — text fields are read from the DOM on demand,
/// which is what keeps the mounted-once panel free of focus loss).
#[derive(Debug, Clone)]
enum Msg {
    Optimize,
    LoadSample,
    Clear,
    #[cfg(feature = "pro")]
    Print,
    #[cfg(feature = "pro")]
    SaveSet,
    #[cfg(feature = "pro")]
    LoadSet(String),
    #[cfg(feature = "pro")]
    DeleteSet(String),
}

const BASE_CSS: &str = r#"
.cutlist-app{--cl-bg:#ffffff;--cl-panel:#f6f7f9;--cl-fg:#1f2430;--cl-muted:#667085;--cl-border:#d9dee7;--cl-accent:#0e7c6b;--cl-accent-fg:#ffffff;--cl-err:#b3261e;--cl-err-bg:#fbeeec;--cl-track:#e4e8ef;--cl-waste:#c9ced9;max-width:880px;color:var(--cl-fg);font-size:15px;line-height:1.5}
@media(prefers-color-scheme:dark){.cutlist-app{--cl-bg:#10151f;--cl-panel:#161d2a;--cl-fg:#e6e9ef;--cl-muted:#98a2b3;--cl-border:#2a3342;--cl-accent:#34b39a;--cl-accent-fg:#0b1f1a;--cl-err:#f2b8b5;--cl-err-bg:#2c1a1c;--cl-track:#1d2634;--cl-waste:#39445a}}
.cutlist-app *{box-sizing:border-box}
.cutlist-app h2{font-size:1.3rem;margin:0 0 2px}
.cutlist-app h3{font-size:1rem;margin:0 0 6px}
.cutlist-sub{color:var(--cl-muted);margin:0 0 14px;font-size:.9rem}
.cutlist-card{background:var(--cl-panel);border:1px solid var(--cl-border);border-radius:10px;padding:14px 16px;margin:0 0 14px}
.cutlist-hint{color:var(--cl-muted);font-size:.82rem;margin:0 0 8px}
.cutlist-parts{width:100%;min-height:150px;padding:9px 10px;border:1px solid var(--cl-border);border-radius:8px;background:var(--cl-bg);color:var(--cl-fg);font:inherit;resize:vertical}
.cutlist-parts:focus,.cutlist-input:focus{outline:2px solid var(--cl-accent);outline-offset:1px}
.cutlist-row{display:flex;gap:14px;flex-wrap:wrap;margin-top:12px}
.cutlist-field{display:flex;flex-direction:column;gap:4px}
.cutlist-label{font-size:.78rem;color:var(--cl-muted)}
.cutlist-input{width:150px;padding:7px 9px;border:1px solid var(--cl-border);border-radius:8px;background:var(--cl-bg);color:var(--cl-fg);font:inherit}
.cutlist-actions{display:flex;gap:8px;flex-wrap:wrap;margin-top:14px}
.cutlist-btn{padding:8px 16px;border-radius:8px;border:1px solid var(--cl-border);background:var(--cl-bg);color:var(--cl-fg);font:inherit;cursor:pointer}
.cutlist-btn:hover{border-color:var(--cl-accent)}
.cutlist-primary{background:var(--cl-accent);border-color:var(--cl-accent);color:var(--cl-accent-fg);font-weight:600}
.cutlist-results-view{margin:4px 0 14px}
.cutlist-summary{margin:0 0 10px}
.cutlist-headline{font-weight:700;font-size:1.02rem;margin:0 0 2px}
.cutlist-detail,.cutlist-engine{color:var(--cl-muted);font-size:.85rem;margin:0}
.cutlist-issues{margin:0 0 12px;padding:10px 12px;background:var(--cl-err-bg);border:1px solid var(--cl-err);border-radius:8px}
.cutlist-issue{color:var(--cl-err);font-size:.88rem;margin:2px 0}
.cutlist-bar{margin:0 0 12px}
.cutlist-bar-head{font-size:.85rem;color:var(--cl-muted);margin:0 0 3px}
.cutlist-track{display:flex;width:100%;height:26px;border-radius:6px;overflow:hidden;background:var(--cl-track)}
.cutlist-seg{height:100%}
.cutlist-c0{background:#4f8ef7}.cutlist-c1{background:#f2a33c}.cutlist-c2{background:#54b26a}.cutlist-c3{background:#e4636f}.cutlist-c4{background:#8f6fd6}.cutlist-c5{background:#2fb6c9}.cutlist-c6{background:#c9a227}.cutlist-c7{background:#7d8896}
.cutlist-waste{background:var(--cl-waste);height:100%}
.cutlist-app table{width:100%;border-collapse:collapse;font-size:.85rem;margin-top:6px}
.cutlist-app th,.cutlist-app td{text-align:left;padding:6px 8px;border-bottom:1px solid var(--cl-border)}
.cutlist-upsell{border:1px dashed var(--cl-border);border-radius:10px;padding:12px 16px;color:var(--cl-muted);font-size:.88rem;margin-top:4px}
.cutlist-upsell a{color:var(--cl-accent);font-weight:600}
.cutlist-stocks{width:100%;min-height:96px;padding:8px 10px;border:1px solid var(--cl-border);border-radius:8px;background:var(--cl-bg);color:var(--cl-fg);font:inherit;resize:vertical}
.cutlist-library{margin:0 0 14px}
.cutlist-lib-title{font-weight:600;font-size:.9rem;margin:0 0 4px}
.cutlist-lib-row{display:flex;gap:8px;align-items:center;padding:3px 0}
.cutlist-lib-name{flex:1;font-size:.88rem;color:var(--cl-fg)}
.cutlist-lib-save{display:flex;gap:8px;margin-top:6px}
.cutlist-lib-empty{color:var(--cl-muted);font-size:.85rem;margin:0}
.cutlist-print{display:none}
@media print{
.cutlist-no-print{display:none!important}
.cutlist-print{display:block}
.cutlist-track{height:20px}
}
"#;

/// Mounts the whole app into `container` (the hub's container div, or the
/// standalone `#tpt-appfront-root` marker).
pub fn mount_app(container: &web_sys::Element) -> Result<(), wasm_bindgen::JsValue> {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    upsert_style(&document, "cutlist-app-styles", BASE_CSS);

    let results = Signal::new(Outcome::Idle);

    // Saved stock sets (Pro): loaded from storage once, then kept in sync
    // by the save/load/delete handlers.
    #[cfg(feature = "pro")]
    let library_sets = Signal::new(crate::library::load());

    // Shared dispatch: button events come back here. Field values are read
    // from the DOM on demand, so the panel is never re-rendered (no focus
    // loss, ever).
    let dispatch: Rc<dyn Fn(Msg)> = {
        let container = container.clone();
        let results = results.clone();
        #[cfg(feature = "pro")]
        let library_sets = library_sets.clone();
        Rc::new(move |msg| match msg {
            Msg::Optimize => run_optimize(&container, &results),
            Msg::LoadSample => {
                write_field(&container, ".cutlist-parts", SAMPLE_PARTS);
                #[cfg(feature = "pro")]
                write_field(&container, ".cutlist-stocks", SAMPLE_STOCKS);
                #[cfg(not(feature = "pro"))]
                write_field(&container, ".cutlist-stock", SAMPLE_STOCK);
                write_field(&container, ".cutlist-kerf", SAMPLE_KERF);
                results.set(Outcome::Idle);
            }
            Msg::Clear => {
                write_field(&container, ".cutlist-parts", "");
                #[cfg(feature = "pro")]
                write_field(&container, ".cutlist-stocks", "");
                #[cfg(not(feature = "pro"))]
                write_field(&container, ".cutlist-stock", SAMPLE_STOCK);
                write_field(&container, ".cutlist-kerf", SAMPLE_KERF);
                results.set(Outcome::Idle);
            }
            #[cfg(feature = "pro")]
            Msg::Print => {
                if let Some(window) = web_sys::window() {
                    let _ = window.print();
                }
            }
            #[cfg(feature = "pro")]
            Msg::SaveSet => save_stock_set(&container, &library_sets),
            #[cfg(feature = "pro")]
            Msg::LoadSet(name) => load_stock_set(&container, &library_sets, &name),
            #[cfg(feature = "pro")]
            Msg::DeleteSet(name) => delete_stock_set(&library_sets, &name),
        })
    };

    // 1. The static shell — mounted exactly once.
    let shell = shell_tree();
    let root = tpt_appfront_dom::mount(container, &shell, dispatch.clone())?;
    std::mem::forget(root);

    // 2. The reactive results section — its own fine-grained render loop
    //    (create_effect + MountedRoot::render) into the placeholder.
    let results_el = container
        .query_selector(".cutlist-results")?
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("results placeholder missing"))?;
    let results_for_view = results.clone();
    let handle = tpt_appfront_dom::render(
        &results_el,
        Rc::new(move || results_view(results_for_view.clone())),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // 3. Pro: the reactive stock-set library and the print-only layout.
    #[cfg(feature = "pro")]
    {
        let library_el = container
            .query_selector(".cutlist-library")?
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("library placeholder missing"))?;
        let handle = tpt_appfront_dom::render(
            &library_el,
            Rc::new(move || library_view(library_sets.clone())),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);

        let print_el = container
            .query_selector(".cutlist-print-root")?
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("print placeholder missing"))?;
        let results_for_print = results.clone();
        let handle = tpt_appfront_dom::render(
            &print_el,
            Rc::new(move || print_view(results_for_print.clone())),
            dispatch,
        )?;
        std::mem::forget(handle);
    }

    #[cfg(not(feature = "pro"))]
    std::mem::forget(dispatch);

    Ok(())
}

/// The mounted-once shell: header, input card, placeholders, upsell.
/// Everything a print should hide carries `.cutlist-no-print`; the Pro
/// placeholders (`.cutlist-library`, `.cutlist-print-root`) receive their
/// own reactive mounts.
fn shell_tree() -> UITree<Msg> {
    // The root's class needs a NodeRef, so build through a ContainerBuilder
    // and extract the single root node (same pattern as the results view).
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            c.heading(2, "Cut List Optimizer")
                .class("cutlist-no-print");
        c.text("Paste your parts, set the stock, optimize — everything runs locally in your browser.")
            .class("cutlist-sub")
            .class("cutlist-no-print");

        c.container(|card| {
            card.heading(3, "1 \u{b7} Job details")
                .class("cutlist-no-print");
            card.text("One part per line: length x quantity, optional label. Tag a material with @name.")
                .class("cutlist-hint");
            card.textarea("").class("cutlist-parts");

            card.container(|fields| {
                #[cfg(feature = "pro")]
                {
                    fields
                        .container(|field| {
                            field
                                .text("Stock lengths — one per line: length [x max bars] [@material]")
                                .class("cutlist-label");
                            field
                                .textarea(SAMPLE_STOCKS)
                                .class("cutlist-stocks");
                        })
                        .class("cutlist-field");
                }
                #[cfg(not(feature = "pro"))]
                {
                    fields
                        .container(|field| {
                            field.text("Stock length (mm)").class("cutlist-label");
                            field
                                .input(SAMPLE_STOCK)
                                .class("cutlist-input cutlist-stock");
                        })
                        .class("cutlist-field");
                }
                fields
                    .container(|field| {
                        field.text("Blade / kerf width (mm)").class("cutlist-label");
                        field
                            .input(SAMPLE_KERF)
                            .class("cutlist-input cutlist-kerf");
                    })
                    .class("cutlist-field");
            })
            .class("cutlist-row");

            card.container(|actions| {
                actions
                    .button("Optimize cuts")
                    .class("cutlist-btn cutlist-primary")
                    .on_click(Msg::Optimize);
                actions
                    .button("Load sample job")
                    .class("cutlist-btn")
                    .on_click(Msg::LoadSample);
                actions
                    .button("Clear")
                    .class("cutlist-btn")
                    .on_click(Msg::Clear);
                #[cfg(feature = "pro")]
                actions
                    .button("Print layout")
                    .class("cutlist-btn")
                    .on_click(Msg::Print);
            })
            .class("cutlist-actions");
        })
        .class("cutlist-card cutlist-no-print");

        // Pro: the reactive saved-stock-set library.
        #[cfg(feature = "pro")]
        c.container(|_| {}).class("cutlist-library");

        // The reactive results section mounts into this placeholder.
        c.container(|_| {}).class("cutlist-results cutlist-no-print");

        // Pro: the print-only shop-floor layout mounts here.
        #[cfg(feature = "pro")]
        c.container(|_| {}).class("cutlist-print-root");

        c.container(|upsell| {
            upsell.heading(3, "Need more than one stock length?");
            upsell.text(
                "The Pro edition ($49) adds multi-stock and multi-material jobs, the improved \
                 optimization search, saved stock-set libraries, and print-ready shop-floor \
                 layout export.",
            );
            upsell.link(
                "https://tptsolutions.co.nz/tools/cutlist-optimizer",
                "Get the Pro edition",
            );
        })
        .class("cutlist-upsell cutlist-no-print");
        });
    root_builder
        .into_only_child()
        .expect("shell builds exactly one root container")
}

/// Reads the input fields, validates, runs the solver, and publishes the
/// outcome. Runs inside the click handler (not inside an effect), so the
/// 200 ms Pro search is off the reactive graph.
fn run_optimize(container: &web_sys::Element, results: &Signal<Outcome>) {
    let parts_text = read_textarea(container, ".cutlist-parts");
    let kerf_text = read_input(container, ".cutlist-kerf");

    let mut issues: Vec<Issue> = Vec::new();

    let kerf = match kerf_text.trim().parse::<u32>() {
        Ok(value) if value <= 50 => value,
        _ => {
            issues.push(Issue {
                line: None,
                message: "Kerf width must be a whole number of millimetres (0-50).".to_string(),
            });
            0
        }
    };

    let parts = match parse::parse_parts(&parts_text) {
        Ok(parts) => parts,
        Err(line_issues) => {
            issues.extend(line_issues);
            Vec::new()
        }
    };

    // Edition-specific stock collection with inline validation.
    #[cfg(feature = "pro")]
    let (stocks, material_names) = collect_pro_stocks(container, &parts, &mut issues);
    #[cfg(not(feature = "pro"))]
    let (stocks, material_names) = collect_free_stock(container, &parts, &mut issues);

    if !issues.is_empty() {
        results.set(Outcome::Invalid(issues));
        return;
    }

    let problem = Problem::new(
        parts
            .iter()
            .map(|p| {
                Part::with_material(
                    p.length,
                    p.qty,
                    p.label.clone(),
                    material_index(&material_names, &p.material),
                )
            })
            .collect(),
        stocks.clone(),
        kerf,
        0,
    );

    match run_solver(&problem) {
        Ok(solution) => results.set(Outcome::Solved {
            solution,
            parts,
            stocks,
            material_names,
            kerf,
        }),
        Err(error) => results.set(Outcome::Invalid(vec![Issue {
            line: None,
            message: error.to_string(),
        }])),
    }
}

/// Free edition: a single stock length field, all parts of one material.
#[cfg(not(feature = "pro"))]
fn collect_free_stock(
    container: &web_sys::Element,
    parts: &[ParsedPart],
    issues: &mut Vec<Issue>,
) -> (Vec<StockSpec>, Vec<String>) {
    let stock_text = read_input(container, ".cutlist-stock");
    let stock_length = match stock_text.trim().parse::<u32>() {
        Ok(value) if value > 0 => value,
        _ => {
            issues.push(Issue {
                line: None,
                message: "Stock length must be a whole number of millimetres (e.g. 2400)."
                    .to_string(),
            });
            0
        }
    };

    // Inline "part longer than stock" validation (the solver guards this
    // too, but this way every issue shows at once, with line numbers).
    if stock_length > 0 {
        for part in parts {
            if part.length > stock_length {
                push_part_too_long(issues, part, stock_length);
            }
        }
    }

    (vec![StockSpec::new(stock_length.max(1))], vec![String::new()])
}

/// Pro edition: the stock textarea, with materials and per-stock bar caps.
#[cfg(feature = "pro")]
fn collect_pro_stocks(
    container: &web_sys::Element,
    parts: &[ParsedPart],
    issues: &mut Vec<Issue>,
) -> (Vec<StockSpec>, Vec<String>) {
    let stocks_text = read_textarea(container, ".cutlist-stocks");
    let parsed_stocks = match parse::parse_stocks(&stocks_text) {
        Ok(stocks) => stocks,
        Err(mut line_issues) => {
            issues.append(&mut line_issues);
            return (Vec::new(), Vec::new());
        }
    };

    // Material registry: index 0 is the unnamed default, then first-appearance
    // order across the stocks and the parts.
    let mut names: Vec<String> = vec![String::new()];
    for stock in &parsed_stocks {
        if !stock.material.is_empty() && !names.contains(&stock.material) {
            names.push(stock.material.clone());
        }
    }
    for part in parts {
        if !part.material.is_empty() && !names.contains(&part.material) {
            names.push(part.material.clone());
        }
    }
    let lookup = |name: &str| -> usize { names.iter().position(|n| n == name).unwrap_or(0) };

    // Every part needs at least one same-material stock it fits on.
    for part in parts {
        let material_label = if part.material.is_empty() {
            "the default material".to_string()
        } else {
            format!("material '{}'", part.material)
        };
        let matching: Vec<_> = parsed_stocks
            .iter()
            .filter(|s| s.material == part.material)
            .collect();
        if matching.is_empty() {
            let name = if part.label.trim().is_empty() {
                format!("on line {}", part.line)
            } else {
                format!("'{}' on line {}", part.label, part.line)
            };
            issues.push(Issue {
                line: Some(part.line),
                message: format!(
                    "part {} needs {} but no stock line defines it",
                    name, material_label
                ),
            });
        } else if !matching.iter().any(|s| s.length >= part.length) {
            push_part_too_long(issues, part, matching.iter().map(|s| s.length).max().unwrap());
        }
    }

    let specs = parsed_stocks
        .iter()
        .map(|s| {
            let spec = StockSpec::new(s.length).material(lookup(&s.material));
            match s.max_bars {
                Some(max) => spec.max_bars(max),
                None => spec,
            }
        })
        .collect();

    (specs, names)
}

/// Inline "part longer than stock" message with the part's line number.
fn push_part_too_long(issues: &mut Vec<Issue>, part: &ParsedPart, longest_stock: u32) {
    let name = if part.label.trim().is_empty() {
        format!("on line {}", part.line)
    } else {
        format!("'{}' on line {}", part.label, part.line)
    };
    issues.push(Issue {
        line: Some(part.line),
        message: format!(
            "part {} ({} mm) is longer than the {} mm stock",
            name, part.length, longest_stock
        ),
    });
}

/// Maps a material name to its dense id (0 = unnamed default).
#[cfg(not(feature = "pro"))]
fn material_index(_names: &[String], _name: &str) -> usize {
    0
}

/// Maps a material name to its dense id (0 = unnamed default).
#[cfg(feature = "pro")]
fn material_index(names: &[String], name: &str) -> usize {
    names.iter().position(|n| n == name).unwrap_or(0)
}

/// Strategy selection happens at compile time: the free edition ships the
/// deterministic baseline only; `--features pro` compiles the improvement
/// search in with a 200 ms budget.
fn run_solver(problem: &Problem) -> Result<Solution, tpt_cutlist_solver::SolveError> {
    #[cfg(feature = "pro")]
    {
        let options = tpt_cutlist_solver::ImproveOptions {
            deadline: Some(Instant::now() + IMPROVE_BUDGET),
            ..tpt_cutlist_solver::ImproveOptions::default()
        };
        tpt_cutlist_solver::solve_optimized(problem, &options)
    }
    #[cfg(not(feature = "pro"))]
    {
        tpt_cutlist_solver::solve(problem)
    }
}

fn display_label(label: &str, part_index: usize) -> String {
    if label.trim().is_empty() {
        format!("Part {}", part_index + 1)
    } else {
        label.to_string()
    }
}

fn read_input(container: &web_sys::Element, selector: &str) -> String {
    container
        .query_selector(selector)
        .ok()
        .flatten()
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.value())
        .unwrap_or_default()
}

fn read_textarea(container: &web_sys::Element, selector: &str) -> String {
    container
        .query_selector(selector)
        .ok()
        .flatten()
        .and_then(|el| el.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
        .map(|area| area.value())
        .unwrap_or_default()
}

fn write_field(container: &web_sys::Element, selector: &str, value: &str) {
    if let Some(el) = container.query_selector(selector).ok().flatten() {
        if let Some(input) = el.dyn_ref::<web_sys::HtmlInputElement>() {
            input.set_value(value);
        } else if let Some(area) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            area.set_value(value);
        }
    }
}

/// Builds the results section tree. The child shape is invariant —
/// [summary container, errors container, bars container, data grid] — so the
/// positional reconciler can always update in place:
/// - summary holds exactly three `text` children,
/// - errors/bars hold any number of same-kind children (extra old children
///   are simply unmounted),
/// - each bar holds exactly [text, track container] with same-kind segments.
fn results_view(outcome: Signal<Outcome>) -> UITree<Msg> {
    let outcome = outcome.get();

    // Proportional widths are classes, so refresh the generated width rules
    // for the tenths this render needs before returning the tree.
    let tenths_used = width_tenths(&outcome);
    if !tenths_used.is_empty() {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            update_width_styles(&document, &tenths_used);
        }
    }

    // The root's class needs a NodeRef, so build through a ContainerBuilder
    // and extract the single root node.
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            c.container(|summary| {
            let (headline, detail) = match &outcome {
                Outcome::Idle => (
                    "Enter your cut list, then press Optimize cuts.".to_string(),
                    String::new(),
                ),
                Outcome::Invalid(issues) => (
                    format!(
                        "Fix {} issue{} before optimizing:",
                        issues.len(),
                        if issues.len() == 1 { "" } else { "s" }
                    ),
                    String::new(),
                ),
                Outcome::Solved {
                    solution,
                    stocks,
                    kerf,
                    ..
                } => {
                    let stock_total: u64 = solution
                        .bars
                        .iter()
                        .map(|bar| u64::from(stocks[bar.stock_index].length))
                        .sum();
                    let headline = format!(
                        "{} bar{} used \u{b7} {} mm of stock \u{b7} {:.1}% utilization",
                        solution.bar_count(),
                        if solution.bar_count() == 1 { "" } else { "s" },
                        grouped(stock_total),
                        solution.utilization * 100.0
                    );
                    #[cfg(feature = "pro")]
                    let headline =
                        format!("{headline} \u{b7} cost {}", grouped(solution.cost));
                    (
                        headline,
                        format!(
                            "Requested {} mm of parts \u{b7} {} mm total offcut (kerf {} mm between cuts)",
                            grouped(solution.parts_length() as u64),
                            grouped(solution.waste as u64),
                            kerf
                        ),
                    )
                }
            };
            summary.text(headline).class("cutlist-headline");
            summary.text(detail).class("cutlist-detail");
            summary.text(ENGINE_NOTE).class("cutlist-engine");
        })
        .class("cutlist-summary");

        c.container(|errors| {
            if let Outcome::Invalid(issues) = &outcome {
                for issue in issues {
                    let prefix = match issue.line {
                        Some(line) => format!("Line {line}: "),
                        None => String::new(),
                    };
                    errors
                        .text(format!("{prefix}{}", issue.message))
                        .class("cutlist-issue");
                }
            }
        })
        .class("cutlist-issues");

        append_bars(c, &outcome);

        #[cfg(feature = "pro")]
        c.data_grid(
            ["Part", "Material", "Length", "Bar", "Position"],
            placement_rows(&outcome),
        );
        #[cfg(not(feature = "pro"))]
        c.data_grid(["Part", "Length", "Bar", "Position"], placement_rows(&outcome));
        })
        .class("cutlist-results-view");
    root_builder
        .into_only_child()
        .expect("results view builds exactly one root container")
}

/// Appends one proportional cut diagram per bar (children of the bars
/// container — all containers, so reconciliation stays in-place).
#[cfg_attr(not(feature = "pro"), allow(unused_variables))]
fn append_bars(c: &mut tpt_appfront_core::ContainerBuilder<Msg>, outcome: &Outcome) {
    let Outcome::Solved {
        solution,
        stocks,
        material_names,
        kerf,
        ..
    } = outcome
    else {
        return;
    };
    for (bar_index, bar) in solution.bars.iter().enumerate() {
        let stock = &stocks[bar.stock_index];
        let stock_length = stock.length;
        let used = bar.parts_length() as u64 + bar.kerf_length(*kerf) as u64;
        let waste = bar.waste(stock_length, *kerf, 0) as u64;

        let header = format!(
            "Bar {} \u{b7} {} cuts \u{b7} {} / {} mm used \u{b7} {} mm offcut",
            bar_index + 1,
            bar.cut_count(),
            grouped(used),
            grouped(stock_length as u64),
            grouped(waste)
        );
        #[cfg(feature = "pro")]
        let header = match material_names.get(stock.material).filter(|n| !n.is_empty()) {
            Some(name) => format!("{header} \u{b7} {name}"),
            None => header,
        };

        c.container(|bar_node| {
            bar_node.text(header).class("cutlist-bar-head");
            bar_node.container(|track| {
                for cut in &bar.cuts {
                    let tenths = cut_tenths(cut.length, stock_length);
                    let color = cut.part % 8;
                    track
                        .container(|_| {})
                        .class(format!("cutlist-seg cutlist-c{color} cutlist-w-{tenths}"));
                }
                let spent: u32 = bar
                    .cuts
                    .iter()
                    .map(|cut| u32::from(cut_tenths(cut.length, stock_length)))
                    .sum();
                let waste_tenths = 1000u32.saturating_sub(spent.min(1000));
                track
                    .container(|_| {})
                    .class(format!("cutlist-waste cutlist-w-{waste_tenths}"));
            })
            .class("cutlist-track");
        })
        .class("cutlist-bar");
    }
}

/// One row per placed cut for the placement table.
#[cfg_attr(not(feature = "pro"), allow(unused_variables))]
fn placement_rows(outcome: &Outcome) -> Vec<Vec<String>> {
    let Outcome::Solved {
        solution,
        parts,
        stocks,
        material_names,
        ..
    } = outcome
    else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (bar_index, bar) in solution.bars.iter().enumerate() {
        for cut in &bar.cuts {
            #[allow(unused_mut)]
            let mut row = vec![
                display_label(&parts[cut.part].label, cut.part),
                format!("{} mm", cut.length),
                format!("{}", bar_index + 1),
                format!("{} mm", cut.offset),
            ];
            #[cfg(feature = "pro")]
            {
                let material = stocks[bar.stock_index].material;
                let name = material_names
                    .get(material)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| "—".to_string());
                row.insert(1, name);
            }
            rows.push(row);
        }
    }
    rows
}

/// Segment width in thousandths of the bar (`0..=1000`), used as a class key.
fn cut_tenths(length: u32, stock_length: u32) -> u16 {
    if stock_length == 0 {
        0
    } else {
        ((u64::from(length) * 1000 / u64::from(stock_length)).min(1000)) as u16
    }
}

/// Every tenth class the current outcome renders (for the style rules).
fn width_tenths(outcome: &Outcome) -> BTreeSet<u16> {
    let mut used = BTreeSet::new();
    if let Outcome::Solved { solution, stocks, .. } = outcome {
        for bar in &solution.bars {
            let stock_length = stocks[bar.stock_index].length;
            for cut in &bar.cuts {
                used.insert(cut_tenths(cut.length, stock_length));
            }
            let spent: u32 = bar
                .cuts
                .iter()
                .map(|cut| u32::from(cut_tenths(cut.length, stock_length)))
                .sum();
            used.insert(u16::try_from(1000u32.saturating_sub(spent.min(1000))).unwrap_or(0));
        }
    }
    used
}

/// Rewrites the generated width-class stylesheet (one rule per tenth in use).
fn update_width_styles(document: &web_sys::Document, tenths: &BTreeSet<u16>) {
    let css = tenths
        .iter()
        .map(|&tenth| {
            let whole = tenth / 10;
            let fraction = tenth % 10;
            let percent = if fraction == 0 {
                format!("{whole}%")
            } else {
                format!("{whole}.{fraction}%")
            };
            format!(".cutlist-w-{tenth}{{width:{percent}}}")
        })
        .collect::<Vec<_>>()
        .join("");
    upsert_style(document, "cutlist-bar-styles", &css);
}

/// Creates or updates the `<style id>` in the document head.
fn upsert_style(document: &web_sys::Document, id: &str, css: &str) {
    if let Ok(Some(existing)) = document.query_selector(&format!("style#{id}")) {
        existing.set_text_content(Some(css));
        return;
    }
    let Some(head) = document.head() else {
        return;
    };
    if let Ok(style_el) = document.create_element("style") {
        let _ = style_el.set_attribute("id", id);
        style_el.set_text_content(Some(css));
        let _ = head.append_child(&style_el);
    }
}

/// Thousands-separated integer, e.g. 19240 -> "19,240".
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

/// Pro: the print-only shop-floor layout. Lives in its own reactive mount
/// (hidden on screen via `.cutlist-print`, revealed by the print stylesheet
/// when the "Print layout" button calls `window.print()`). Mirrors the
/// results section so it is always in sync.
#[cfg(feature = "pro")]
fn print_view(results: Signal<Outcome>) -> UITree<Msg> {
    let outcome = results.get();

    // The print layout reuses the diagram width classes — make sure the
    // rules for this solution exist before returning the tree.
    let tenths_used = width_tenths(&outcome);
    if !tenths_used.is_empty() {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            update_width_styles(&document, &tenths_used);
        }
    }

    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            c.heading(2, "Cut List \u{b7} Shop-Floor Layout");
            let Outcome::Solved {
                solution,
                stocks,
                kerf,
                ..
            } = &outcome
            else {
                c.text("Optimize a job first — the print layout mirrors the results.")
                    .class("cutlist-engine");
                c.text("");
                c.text("");
                c.container(|_| {});
                c.data_grid(
                    ["Part", "Length", "Bar", "Position"],
                    Vec::<Vec<String>>::new(),
                );
                return;
            };

            let stock_total: u64 = solution
                .bars
                .iter()
                .map(|bar| u64::from(stocks[bar.stock_index].length))
                .sum();
            c.text(format!(
                "{} bar{} \u{b7} {} mm stock \u{b7} {} mm parts \u{b7} {} mm offcut \u{b7} kerf {} mm",
                solution.bar_count(),
                if solution.bar_count() == 1 { "" } else { "s" },
                grouped(stock_total),
                grouped(solution.parts_length() as u64),
                grouped(solution.waste as u64),
                kerf
            ))
            .class("cutlist-detail");
            c.text(ENGINE_NOTE).class("cutlist-engine");

            append_bars(c, &outcome);

            c.data_grid(
                ["Part", "Material", "Length", "Bar", "Position"],
                placement_rows(&outcome),
            );
        })
        .class("cutlist-print");
    root_builder
        .into_only_child()
        .expect("print view builds exactly one root container")
}

/// Pro: the saved stock-set library panel.
#[cfg(feature = "pro")]
fn library_view(sets: Signal<Vec<SavedSet>>) -> UITree<Msg> {
    let sets = sets.get();
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            c.heading(3, "2 \u{b7} Saved stock sets");
            c.container(|rows| {
                if sets.is_empty() {
                    rows.text("No saved sets yet — configure the stock lengths above and save them for next time.")
                        .class("cutlist-lib-empty");
                }
                for set in &sets {
                    rows.container(|row| {
                        row.text(set.name.clone()).class("cutlist-lib-name");
                        row.button("Load")
                            .class("cutlist-btn")
                            .on_click(Msg::LoadSet(set.name.clone()));
                        row.button("Delete")
                            .class("cutlist-btn")
                            .on_click(Msg::DeleteSet(set.name.clone()));
                    })
                    .class("cutlist-lib-row");
                }
            })
            .class("cutlist-lib-rows");
            c.container(|save_row| {
                save_row.input("").class("cutlist-input cutlib-name");
                save_row
                    .button("Save current stock set")
                    .class("cutlist-btn")
                    .on_click(Msg::SaveSet);
            })
            .class("cutlist-lib-save");
        })
        .class("cutlist-library");
    root_builder
        .into_only_child()
        .expect("library view builds exactly one root container")
}

/// Pro: saves the current stock textarea + kerf under a name.
#[cfg(feature = "pro")]
fn save_stock_set(container: &web_sys::Element, sets: &Signal<Vec<SavedSet>>) {
    let name = read_input(container, ".cutlib-name").trim().to_string();
    let stocks = read_textarea(container, ".cutlist-stocks");
    if name.is_empty() || stocks.trim().is_empty() {
        return;
    }
    let kerf = read_input(container, ".cutlist-kerf");
    let mut list = sets.get();
    list.retain(|set| set.name != name);
    list.push(SavedSet { name, stocks, kerf });
    list.sort_by(|a, b| a.name.cmp(&b.name));
    crate::library::store(&list);
    sets.set(list);
}

/// Pro: loads a saved set into the stock textarea + kerf field.
#[cfg(feature = "pro")]
fn load_stock_set(container: &web_sys::Element, sets: &Signal<Vec<SavedSet>>, name: &str) {
    if let Some(set) = sets.get().into_iter().find(|set| set.name == name) {
        write_field(container, ".cutlist-stocks", &set.stocks);
        write_field(container, ".cutlist-kerf", &set.kerf);
    }
}

/// Pro: deletes a saved set.
#[cfg(feature = "pro")]
fn delete_stock_set(sets: &Signal<Vec<SavedSet>>, name: &str) {
    let mut list = sets.get();
    list.retain(|set| set.name != name);
    crate::library::store(&list);
    sets.set(list);
}




