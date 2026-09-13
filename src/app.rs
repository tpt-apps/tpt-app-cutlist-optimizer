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
use std::time::{Duration, Instant};

/// The Pro build runs the improvement search with a hard wall-clock budget.
#[cfg(feature = "pro")]
const IMPROVE_BUDGET: Duration = Duration::from_millis(200);

#[cfg(feature = "pro")]
const ENGINE_NOTE: &str =
    "Optimization: best-fit-decreasing + randomized improvement search (Pro edition).";

#[cfg(not(feature = "pro"))]
const ENGINE_NOTE: &str =
    "Optimization: best-fit-decreasing (free edition — the paid Pro build adds an improved search).";

const SAMPLE_PARTS: &str = "730 x 4 Cabinet side\n560 x 6 Shelf\n396 x 4 Top rail\n197 x 8 Door stile\n640 x 2 Door panel";
const SAMPLE_STOCK: &str = "2400";
const SAMPLE_KERF: &str = "3";

/// What the results section is currently showing.
#[derive(Clone)]
enum Outcome {
    Idle,
    Invalid(Vec<Issue>),
    Solved {
        solution: Solution,
        parts: Vec<ParsedPart>,
        stock_length: u32,
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
"#;

/// Mounts the whole free-edition app into `container` (the hub's container
/// div, or the standalone `#tpt-appfront-root` marker).
pub fn mount_app(container: &web_sys::Element) -> Result<(), wasm_bindgen::JsValue> {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    upsert_style(&document, "cutlist-app-styles", BASE_CSS);

    let results = Signal::new(Outcome::Idle);

    // Shared dispatch: button events come back here. Field values are read
    // from the DOM at Optimize time, so no on_input wiring is needed and the
    // panel is never re-rendered (no focus loss, ever).
    let dispatch: Rc<dyn Fn(Msg)> = {
        let container = container.clone();
        let results = results.clone();
        Rc::new(move |msg| match msg {
            Msg::Optimize => run_optimize(&container, &results),
            Msg::LoadSample => {
                write_field(&container, ".cutlist-parts", SAMPLE_PARTS);
                write_field(&container, ".cutlist-stock", SAMPLE_STOCK);
                write_field(&container, ".cutlist-kerf", SAMPLE_KERF);
                results.set(Outcome::Idle);
            }
            Msg::Clear => {
                write_field(&container, ".cutlist-parts", "");
                write_field(&container, ".cutlist-stock", SAMPLE_STOCK);
                write_field(&container, ".cutlist-kerf", SAMPLE_KERF);
                results.set(Outcome::Idle);
            }
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
    let handle = tpt_appfront_dom::render(
        &results_el,
        Rc::new(move || results_view(results.clone())),
        dispatch,
    )?;
    std::mem::forget(handle);

    Ok(())
}

/// The mounted-once shell: header, input card, results placeholder, upsell.
fn shell_tree() -> UITree<Msg> {
    UITree::container(|c| {
        c.heading(2, "Cut List Optimizer");
        c.text("Paste your parts, set the stock, optimize — everything runs locally in your browser.")
            .class("cutlist-sub");

        c.container(|card| {
            card.heading(3, "1 \u{b7} Job details");
            card.text("One part per line: length x quantity, optional label.")
                .class("cutlist-hint");
            card.textarea("").class("cutlist-parts");

            card.container(|row| {
                row.container(|field| {
                    field.text("Stock length (mm)").class("cutlist-label");
                    field
                        .input(SAMPLE_STOCK)
                        .class("cutlist-input cutlist-stock");
                })
                .class("cutlist-field");
                row.container(|field| {
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
            })
            .class("cutlist-actions");
        })
        .class("cutlist-card");

        // The reactive results section mounts into this placeholder.
        c.container(|_| {}).class("cutlist-results");

        c.container(|upsell| {
            upsell.heading(3, "Need more than one stock length?");
            upsell.text(
                "The Pro edition ($49) adds multi-stock and multi-material jobs, the improved \
                 optimization search, saved material libraries, and print-ready shop-floor \
                 layout export.",
            );
            upsell.link(
                "https://tptsolutions.co.nz/tools/cutlist-optimizer",
                "Get the Pro edition",
            );
        })
        .class("cutlist-upsell");
    })
}

/// Reads the input fields, validates, runs the solver, and publishes the
/// outcome. Runs inside the click handler (not inside an effect), so the
/// 200 ms Pro search is off the reactive graph.
fn run_optimize(container: &web_sys::Element, results: &Signal<Outcome>) {
    let parts_text = read_textarea(container, ".cutlist-parts");
    let stock_text = read_input(container, ".cutlist-stock");
    let kerf_text = read_input(container, ".cutlist-kerf");

    let mut issues: Vec<Issue> = Vec::new();

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

    // Inline "part longer than stock" validation (the solver guards this too,
    // but this way every issue shows at once, with line numbers).
    if stock_length > 0 {
        for part in &parts {
            if part.length > stock_length {
                let name = if part.label.trim().is_empty() {
                    format!("on line {}", part.line)
                } else {
                    format!("'{}' on line {}", part.label, part.line)
                };
                issues.push(Issue {
                    line: Some(part.line),
                    message: format!(
                        "part {} ({} mm) is longer than the {} mm stock",
                        name, part.length, stock_length
                    ),
                });
            }
        }
    }

    if !issues.is_empty() {
        results.set(Outcome::Invalid(issues));
        return;
    }

    let problem = Problem::new(
        parts
            .iter()
            .map(|p| Part::new(p.length, p.qty, p.label.clone()))
            .collect(),
        StockSpec::new(stock_length),
        kerf,
        0,
    );

    let solved = run_solver(&problem);
    match solved {
        Ok(solution) => results.set(Outcome::Solved {
            solution,
            parts,
            stock_length,
            kerf,
        }),
        Err(error) => results.set(Outcome::Invalid(vec![Issue {
            line: None,
            message: error.to_string(),
        }])),
    }
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
                    stock_length,
                    kerf,
                    ..
                } => {
                    let stock_total = solution.bar_count() as u64 * *stock_length as u64;
                    (
                        format!(
                            "{} bar{} used \u{b7} {} mm of stock \u{b7} {:.1}% utilization",
                            solution.bar_count(),
                            if solution.bar_count() == 1 { "" } else { "s" },
                            grouped(stock_total),
                            solution.utilization * 100.0
                        ),
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

        c.data_grid(["Part", "Length", "Bar", "Position"], placement_rows(&outcome));
        })
        .class("cutlist-results-view");
    root_builder
        .into_only_child()
        .expect("results view builds exactly one root container")
}

/// Appends one proportional cut diagram per bar (children of the bars
/// container — all containers, so reconciliation stays in-place).
fn append_bars(c: &mut tpt_appfront_core::ContainerBuilder<Msg>, outcome: &Outcome) {
    let Outcome::Solved {
        solution,
        stock_length,
        kerf,
        ..
    } = outcome
    else {
        return;
    };
    for (bar_index, bar) in solution.bars.iter().enumerate() {
        let used = bar.parts_length() as u64 + bar.kerf_length(*kerf) as u64;
        let waste = bar.waste(*stock_length, *kerf, 0) as u64;
        c.container(|bar_node| {
            bar_node
                .text(format!(
                    "Bar {} \u{b7} {} cuts \u{b7} {} / {} mm used \u{b7} {} mm offcut",
                    bar_index + 1,
                    bar.cut_count(),
                    grouped(used),
                    grouped(*stock_length as u64),
                    grouped(waste)
                ))
                .class("cutlist-bar-head");
            bar_node.container(|track| {
                for cut in &bar.cuts {
                    let tenths = cut_tenths(cut.length, *stock_length);
                    let color = cut.part % 8;
                    track
                        .container(|_| {})
                        .class(format!("cutlist-seg cutlist-c{color} cutlist-w-{tenths}"));
                }
                let spent: u32 = bar
                    .cuts
                    .iter()
                    .map(|cut| u32::from(cut_tenths(cut.length, *stock_length)))
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
fn placement_rows(outcome: &Outcome) -> Vec<Vec<String>> {
    let Outcome::Solved { solution, parts, .. } = outcome else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (bar_index, bar) in solution.bars.iter().enumerate() {
        for cut in &bar.cuts {
            rows.push(vec![
                display_label(&parts[cut.part].label, cut.part),
                format!("{} mm", cut.length),
                format!("{}", bar_index + 1),
                format!("{} mm", cut.offset),
            ]);
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
    if let Outcome::Solved {
        solution,
        stock_length,
        ..
    } = outcome
    {
        for bar in &solution.bars {
            for cut in &bar.cuts {
                used.insert(cut_tenths(cut.length, *stock_length));
            }
            let spent: u32 = bar
                .cuts
                .iter()
                .map(|cut| u32::from(cut_tenths(cut.length, *stock_length)))
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




