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
//! The DOM backend applies `class` verbatim, so elements are hooked by
//! unique class names, proportional bar widths use generated
//! `.cutlist-w-<tenths>` classes backed by an injected `<style>`, and the
//! whole stylesheet is injected once at mount — no external CSS.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use tpt_appfront_core::{Signal, UITree};
use tpt_cutlist_solver::{Part, Problem, Solution, StockSpec};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use crate::examples::{decode_job, encode_job, EXAMPLES};
use crate::parse::{
    self, csv_to_parts_lines, fmt_len, fmt_len_plain, grouped, parts_to_lines, Issue, ParsedPart,
    Units,
};
use crate::project::{self, ProjectJob};

#[cfg(feature = "pro")]
use crate::library::SavedSet;
#[cfg(feature = "pro")]
use crate::parse::stocks_to_lines;
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

/// Free edition: the Pro nudge appears only after this many optimizations
/// on the device, instead of on first load (softer funnel).
#[cfg(not(feature = "pro"))]
const NUDGE_AFTER: u32 = 3;

/// Free edition: the offcut-inventory threshold is fixed (Pro gets a field).
#[cfg(not(feature = "pro"))]
const FREE_MIN_OFFCUT: u32 = 150;

/// Permalinks longer than this are not written into the URL (browsers and
/// chat apps choke on huge links); the stored job still works offline.
const MAX_HASH_LEN: usize = 6000;

fn sample_parts() -> &'static str {
    EXAMPLES[0].1
}

fn sample_stock() -> &'static str {
    EXAMPLES[0].2
}

fn sample_kerf() -> &'static str {
    EXAMPLES[0].3
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

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

/// Kerf calibration wizard state ("cut twice, measure the gap").
#[derive(Clone)]
struct KerfWiz {
    open: bool,
    /// Raw field contents, echoed back so re-renders never wipe typing.
    before: String,
    piece: String,
    offcut: String,
    message: String,
    /// The computed kerf once the measurements add up.
    kerf_mm: Option<u32>,
}

impl Default for KerfWiz {
    fn default() -> Self {
        KerfWiz {
            open: false,
            before: String::new(),
            piece: String::new(),
            offcut: String::new(),
            message: String::new(),
            kerf_mm: None,
        }
    }
}

/// App events (buttons only — text fields are read from the DOM on demand,
/// which is what keeps the mounted-once panel free of focus loss). The parts
/// and stock textareas are the exception: their `on_input` only feeds the
/// live counters, never a re-render of the panel itself.
#[derive(Debug, Clone)]
enum Msg {
    Optimize,
    Clear,
    LoadExample(usize),
    PartsChanged(String),
    ToggleHelp,
    SetUnits(String),
    ExportCsv,
    ExportSvg,
    CopyLink,
    ImportCsv,
    SaveProject,
    OpenProject,
    ToggleKerfWiz,
    CalcKerf,
    UseKerf,
    #[cfg(feature = "pro")]
    Print,
    #[cfg(feature = "pro")]
    StocksChanged(String),
    #[cfg(feature = "pro")]
    SaveSet,
    #[cfg(feature = "pro")]
    LoadSet(String),
    #[cfg(feature = "pro")]
    DeleteSet(String),
    #[cfg(feature = "pro")]
    SaveOffcuts,
    #[cfg(feature = "pro")]
    OffcutMinChanged(String),
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
.cutlist-actions{display:flex;gap:8px;flex-wrap:wrap;margin-top:14px;align-items:center}
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
.cutlist-upsell-hidden{margin:0;padding:0;border:none}
.cutlist-stocks{width:100%;min-height:96px;padding:8px 10px;border:1px solid var(--cl-border);border-radius:8px;background:var(--cl-bg);color:var(--cl-fg);font:inherit;resize:vertical}
.cutlist-library{margin:0 0 14px}
.cutlist-lib-title{font-weight:600;font-size:.9rem;margin:0 0 4px}
.cutlist-lib-row{display:flex;gap:8px;align-items:center;padding:3px 0}
.cutlist-lib-name{flex:1;font-size:.88rem;color:var(--cl-fg)}
.cutlist-lib-save{display:flex;gap:8px;margin-top:6px}
.cutlist-lib-empty{color:var(--cl-muted);font-size:.85rem;margin:0}
.cutlist-print{display:none}
.cutlist-header{display:flex;justify-content:space-between;align-items:baseline;gap:10px;flex-wrap:wrap}
.cutlist-ghost{background:transparent}
.cutlist-examples{display:flex;gap:6px;flex-wrap:wrap;margin-top:8px;align-items:center}
.cutlist-live{color:var(--cl-muted);font-size:.8rem;margin:6px 0 0;min-height:1.1em}
.cutlist-divider{border-top:1px solid var(--cl-border);margin:14px 0 12px}
.cutlist-stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:8px;margin:0 0 12px}
.cutlist-stat{background:var(--cl-panel);border:1px solid var(--cl-border);border-radius:8px;padding:8px 10px}
.cutlist-stat-value{font-weight:700;font-size:1.15rem;margin:0}
.cutlist-stat-label{color:var(--cl-muted);font-size:.75rem;margin:0}
.cutlist-legend{display:flex;gap:10px;flex-wrap:wrap;margin-top:4px}
.cutlist-legend-item{display:flex;gap:5px;align-items:center}
.cutlist-swatch{width:12px;height:12px;border-radius:3px;flex:none}
.cutlist-legend-label{font-size:.78rem;color:var(--cl-muted);margin:0}
.cutlist-seq-head{font-size:.88rem;font-weight:600;margin:12px 0 2px}
.cutlist-seq-line{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:.78rem;margin:2px 0;color:var(--cl-fg);white-space:pre-wrap}
.cutlist-thresh{width:70px}
.cutlist-help{display:none;background:var(--cl-panel);border:1px solid var(--cl-border);border-radius:10px;padding:12px 16px;margin:0 0 14px}
.cutlist-help.cutlist-open{display:block}
.cutlist-help p{margin:6px 0;font-size:.9rem}
@media print{
.cutlist-no-print{display:none!important}
.cutlist-help-root{display:none!important}
.cutlist-print{display:block}
.cutlist-track{height:20px}
}
"#;

/// Mounts the whole app into `container` (the hub's container div, or the
/// standalone `#tpt-appfront-root` marker).
pub fn mount_app(container: &web_sys::Element) -> Result<(), JsValue> {
    let document = web_sys::window()
        .expect("no window")
        .document()
        .expect("no document");

    upsert_style(&document, "cutlist-app-styles", BASE_CSS);

    let results = Signal::new(Outcome::Idle);

    // Live counters, the collapsible guide, and the selected units.
    let parts_text = Signal::new(String::new());
    #[cfg(feature = "pro")]
    let stocks_text = Signal::new(String::new());
    let help_open = Signal::new(true);
    let units = Signal::new(Units::from_name(
        &storage_get("cutlist-units").unwrap_or_default(),
    ));

    // Upgrade nudge metrics: how many jobs this device has optimized.
    let opt_count = Signal::new(
        storage_get("cutlist-optimize-count")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0),
    );

    // Saved stock sets (Pro): loaded from storage once, then kept in sync
    // by the save/load/delete handlers.
    #[cfg(feature = "pro")]
    let library_sets = Signal::new(crate::library::load());

    // Pro: the "save offcuts as stock" threshold, in millimetres.
    #[cfg(feature = "pro")]
    let offcut_min = Signal::new(
        storage_get("cutlist-offcut-min")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value <= 100_000)
            .unwrap_or(150),
    );

    let kerf_wiz = Signal::new(KerfWiz::default());

    // Shared dispatch: button events come back here. Field values are read
    // from the DOM on demand, so the panel is never re-rendered (no focus
    // loss, ever).
    let dispatch: Rc<dyn Fn(Msg)> = {
        let container = container.clone();
        let results = results.clone();
        let parts_text = parts_text.clone();
        let help_open = help_open.clone();
        let units = units.clone();
        let opt_count = opt_count.clone();
        let kerf_wiz = kerf_wiz.clone();
        #[cfg(feature = "pro")]
        let library_sets = library_sets.clone();
        #[cfg(feature = "pro")]
        let stocks_text = stocks_text.clone();
        #[cfg(feature = "pro")]
        let offcut_min = offcut_min.clone();
        Rc::new(move |msg| match msg {
            Msg::Optimize => run_optimize(&container, &results, &units, &opt_count),
            Msg::LoadExample(index) => {
                let Some((_, parts_mm, stock_mm, kerf_mm)) = EXAMPLES.get(index) else {
                    return;
                };
                let units_now = units.get();
                // Examples are stored canonically in millimetres; show them
                // in whatever unit the UI is currently using.
                let parts_shown = convert_parts_text(parts_mm, Units::Millimetres, units_now);
                #[cfg(feature = "pro")]
                let stock_shown = convert_stocks_text(stock_mm, Units::Millimetres, units_now);
                #[cfg(not(feature = "pro"))]
                let stock_shown = convert_single_text(stock_mm, Units::Millimetres, units_now);
                let kerf_shown = convert_single_text(kerf_mm, Units::Millimetres, units_now);
                write_field(&container, ".cutlist-parts", &parts_shown);
                parts_text.set(parts_shown);
                #[cfg(feature = "pro")]
                {
                    write_field(&container, ".cutlist-stocks", &stock_shown);
                    stocks_text.set(stock_shown);
                }
                #[cfg(not(feature = "pro"))]
                write_field(&container, ".cutlist-stock", &stock_shown);
                write_field(&container, ".cutlist-kerf", &kerf_shown);
                // Loading an example is also the demo: optimize right away so
                // first-time users immediately see a solved cut list.
                run_optimize(&container, &results, &units, &opt_count);
            }
            Msg::Clear => {
                let units_now = units.get();
                write_field(&container, ".cutlist-parts", "");
                parts_text.set(String::new());
                #[cfg(feature = "pro")]
                {
                    write_field(&container, ".cutlist-stocks", "");
                    stocks_text.set(String::new());
                }
                #[cfg(not(feature = "pro"))]
                write_field(
                    &container,
                    ".cutlist-stock",
                    &convert_single_text(sample_stock(), Units::Millimetres, units_now),
                );
                write_field(
                    &container,
                    ".cutlist-kerf",
                    &convert_single_text(sample_kerf(), Units::Millimetres, units_now),
                );
                results.set(Outcome::Idle);
            }
            Msg::PartsChanged(text) => parts_text.set(text),
            Msg::ToggleHelp => help_open.set(!help_open.get()),
            Msg::SetUnits(name) => {
                let new_units = Units::from_name(&name);
                let old_units = units.get();
                if new_units == old_units {
                    return;
                }
                // Convert the loaded job so the same physical pieces stay on
                // screen; a field mid-edit (unparseable) is left untouched.
                convert_dom_field(&container, ".cutlist-parts", old_units, new_units, &parts_text);
                #[cfg(feature = "pro")]
                convert_dom_field(&container, ".cutlist-stocks", old_units, new_units, &stocks_text);
                #[cfg(not(feature = "pro"))]
                {
                    let stock_text = read_input(&container, ".cutlist-stock");
                    if let Some(mm) = parse::parse_len(&stock_text, old_units) {
                        write_field(&container, ".cutlist-stock", &fmt_len_plain(mm, new_units));
                    }
                }
                let kerf_text = read_input(&container, ".cutlist-kerf");
                if let Some(mm) = parse::parse_len(&kerf_text, old_units) {
                    write_field(&container, ".cutlist-kerf", &fmt_len_plain(mm, new_units));
                }
                storage_set("cutlist-units", new_units.name());
                units.set(new_units);
            }
            Msg::ExportCsv => export_csv(&results, &units),
            Msg::ExportSvg => export_svg(&results, &units),
            Msg::CopyLink => copy_share_link(&container, &units),
            Msg::ImportCsv => import_csv(&container, &results, &units, &parts_text, &opt_count),
            #[cfg(feature = "pro")]
            Msg::OpenProject => open_project(
                &container,
                &results,
                &units,
                &parts_text,
                &opt_count,
                &stocks_text,
            ),
            #[cfg(not(feature = "pro"))]
            Msg::OpenProject => open_project(&container, &results, &units, &parts_text, &opt_count),
            Msg::SaveProject => save_project(&container, &units),
            Msg::ToggleKerfWiz => {
                let mut state = kerf_wiz.get();
                state.open = !state.open;
                state.message.clear();
                state.kerf_mm = None;
                kerf_wiz.set(state);
            }
            Msg::CalcKerf => calculate_kerf(&container, &kerf_wiz, &units),
            Msg::UseKerf => {
                let state = kerf_wiz.get();
                if let Some(mm) = state.kerf_mm {
                    write_field(
                        &container,
                        ".cutlist-kerf",
                        &fmt_len_plain(mm, units.get()),
                    );
                    kerf_wiz.set(KerfWiz {
                        open: false,
                        ..state
                    });
                }
            }
            #[cfg(feature = "pro")]
            Msg::StocksChanged(text) => stocks_text.set(text),
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
            #[cfg(feature = "pro")]
            Msg::SaveOffcuts => {
                save_offcuts(&container, &results, &stocks_text, &offcut_min)
            }
            #[cfg(feature = "pro")]
            Msg::OffcutMinChanged(text) => {
                if let Ok(value) = text.trim().parse::<u32>() {
                    let value = value.min(100_000);
                    storage_set("cutlist-offcut-min", &value.to_string());
                    offcut_min.set(value);
                }
            }
        })
    };

    // 1. The static shell — mounted exactly once.
    let shell = shell_tree();
    let root = tpt_appfront_dom::mount(container, &shell, dispatch.clone())?;
    std::mem::forget(root);

    // Ctrl+Enter runs the optimizer from anywhere (including inside the
    // textareas). Raw keydown listener; the closure is intentionally leaked
    // for the lifetime of the page.
    let dispatch_for_keys = dispatch.clone();
    let keydown = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
        move |event: web_sys::KeyboardEvent| {
            if event.ctrl_key() && !event.shift_key() && !event.alt_key() && event.key() == "Enter"
            {
                event.prevent_default();
                dispatch_for_keys(Msg::Optimize);
            }
        },
    );
    document
        .add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref())?;
    keydown.forget();

    // 2. The reactive results section — its own fine-grained render loop
    //    (create_effect + MountedRoot::render) into the placeholder.
    let results_el = container
        .query_selector(".cutlist-results")?
        .ok_or_else(|| JsValue::from_str("results placeholder missing"))?;
    let results_for_view = results.clone();
    let units_for_view = units.clone();
    #[cfg(feature = "pro")]
    let offcut_for_view = offcut_min.clone();
    #[cfg(not(feature = "pro"))]
    let offcut_for_view = Signal::new(FREE_MIN_OFFCUT);
    let dispatch_for_results = dispatch.clone();
    let handle = tpt_appfront_dom::render(
        &results_el,
        Rc::new(move || results_view(results_for_view.clone(), units_for_view.clone(), offcut_for_view.clone())),
        dispatch_for_results,
    )?;
    std::mem::forget(handle);

    // 3. The collapsible "How it works" guide.
    let help_el = container
        .query_selector(".cutlist-help-root")?
        .ok_or_else(|| JsValue::from_str("help placeholder missing"))?;
    let help_for_view = help_open.clone();
    let handle = tpt_appfront_dom::render(
        &help_el,
        Rc::new(move || help_view(help_for_view.clone())),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // 4. Live counters under the textareas (re-parse on every keystroke —
    //    cheap, and only these tiny sections re-render).
    let parts_live_el = container
        .query_selector(".cutlist-parts-live")?
        .ok_or_else(|| JsValue::from_str("parts counter placeholder missing"))?;
    let parts_text_for_live = parts_text.clone();
    let units_for_live = units.clone();
    let dispatch_for_live = dispatch.clone();
    let handle = tpt_appfront_dom::render(
        &parts_live_el,
        Rc::new(move || parts_live_view(parts_text_for_live.clone(), units_for_live.clone())),
        dispatch_for_live,
    )?;
    std::mem::forget(handle);

    // 5. The units selector (its own reactive mount so the selected value
    //    survives signal-driven re-renders).
    let units_el = container
        .query_selector(".cutlist-units-root")?
        .ok_or_else(|| JsValue::from_str("units placeholder missing"))?;
    let units_for_units_view = units.clone();
    let handle = tpt_appfront_dom::render(
        &units_el,
        Rc::new(move || units_view(units_for_units_view.clone())),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // 6. The kerf calibration wizard.
    let kerf_wiz_el = container
        .query_selector(".cutlist-kerf-wiz-root")?
        .ok_or_else(|| JsValue::from_str("kerf wizard placeholder missing"))?;
    let kerf_wiz_for_view = kerf_wiz.clone();
    let units_for_wiz = units.clone();
    let handle = tpt_appfront_dom::render(
        &kerf_wiz_el,
        Rc::new(move || kerf_wiz_view(kerf_wiz_for_view.clone(), units_for_wiz.clone())),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // 7. Free edition: the Pro upsell appears only after a few jobs.
    #[cfg(not(feature = "pro"))]
    {
        let upsell_el = container
            .query_selector(".cutlist-upsell-root")?
            .ok_or_else(|| JsValue::from_str("upsell placeholder missing"))?;
        let opt_count_for_view = opt_count.clone();
        let handle = tpt_appfront_dom::render(
            &upsell_el,
            Rc::new(move || upsell_view(opt_count_for_view.clone())),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);
    }

    // 8. Pro: the reactive stock-set library, live stock counter, and the
    //    print-only layout.
    #[cfg(feature = "pro")]
    {
        let stocks_live_el = container
            .query_selector(".cutlist-stocks-live")?
            .ok_or_else(|| JsValue::from_str("stock counter placeholder missing"))?;
        let stocks_text_for_live = stocks_text.clone();
        let units_for_stocks_live = units.clone();
        let handle = tpt_appfront_dom::render(
            &stocks_live_el,
            Rc::new(move || {
                stocks_live_view(stocks_text_for_live.clone(), units_for_stocks_live.clone())
            }),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);

        let library_el = container
            .query_selector(".cutlist-library")?
            .ok_or_else(|| JsValue::from_str("library placeholder missing"))?;
        let handle = tpt_appfront_dom::render(
            &library_el,
            Rc::new(move || library_view(library_sets.clone())),
            dispatch.clone(),
        )?;
        std::mem::forget(handle);

        let print_el = container
            .query_selector(".cutlist-print-root")?
            .ok_or_else(|| JsValue::from_str("print placeholder missing"))?;
        let results_for_print = results.clone();
        let units_for_print = units.clone();
        let handle = tpt_appfront_dom::render(
            &print_el,
            Rc::new(move || print_view(results_for_print.clone(), units_for_print.clone())),
            dispatch,
        )?;
        std::mem::forget(handle);
    }

    // Restore-on-load runs AFTER every reactive section is wired: a share
    // link (or stored job) solved here triggers exactly one reconcile of
    // the already-mounted (empty) results view. Solving any earlier would
    // bake rows into the initial DOM without mounted records, and the first
    // effect run would duplicate them.
    //
    // A share link (`#/j=…`) wins over the stored last job: whoever opened
    // the link wants THAT job solved, immediately.
    let mut permalink_loaded = false;
    if let Some((parts, stock, kerf, unit)) = read_permalink_job() {
        units.set(Units::from_name(&unit));
        write_field(container, ".cutlist-parts", &parts);
        parts_text.set(parts);
        #[cfg(feature = "pro")]
        {
            write_field(container, ".cutlist-stocks", &stock);
            stocks_text.set(stock);
        }
        #[cfg(not(feature = "pro"))]
        write_field(container, ".cutlist-stock", &stock);
        write_field(container, ".cutlist-kerf", &kerf);
        // The optimizer rewrites a fresh permalink on every solve, so strip
        // the incoming one here to keep refreshes predictable.
        clear_hash();
        run_optimize(container, &results, &units, &opt_count);
        permalink_loaded = true;
    }

    // Otherwise restore the last job (kept on this device only).
    if !permalink_loaded {
        if let Some(parts) = storage_get("cutlist-last-parts") {
            if !parts.trim().is_empty() {
                write_field(container, ".cutlist-parts", &parts);
                parts_text.set(parts);
                if let Some(stock) = storage_get("cutlist-last-stock") {
                    #[cfg(feature = "pro")]
                    {
                        write_field(container, ".cutlist-stocks", &stock);
                        stocks_text.set(stock);
                    }
                    #[cfg(not(feature = "pro"))]
                    write_field(container, ".cutlist-stock", &stock);
                }
                if let Some(kerf) = storage_get("cutlist-last-kerf") {
                    write_field(container, ".cutlist-kerf", &kerf);
                }
            }
        } else {
            // Fresh start: restate the shell's millimetre defaults in the
            // remembered unit.
            let units_now = units.get();
            if units_now != Units::Millimetres {
                let parts_shown = convert_parts_text(sample_parts(), Units::Millimetres, units_now);
                write_field(container, ".cutlist-parts", &parts_shown);
                parts_text.set(parts_shown);
                #[cfg(feature = "pro")]
                {
                    let stock_shown =
                        convert_stocks_text(sample_stock(), Units::Millimetres, units_now);
                    write_field(container, ".cutlist-stocks", &stock_shown);
                    stocks_text.set(stock_shown);
                }
                #[cfg(not(feature = "pro"))]
                write_field(
                    container,
                    ".cutlist-stock",
                    &convert_single_text(sample_stock(), Units::Millimetres, units_now),
                );
                write_field(
                    container,
                    ".cutlist-kerf",
                    &convert_single_text(sample_kerf(), Units::Millimetres, units_now),
                );
            }
        }
    }

    #[cfg(not(feature = "pro"))]
    std::mem::forget(dispatch);

    Ok(())
}

/// The mounted-once shell: header, input card, placeholders. Everything a
/// print should hide carries `.cutlist-no-print`; the placeholders
/// (`.cutlist-results`, `.cutlist-units-root`, …) receive their own
/// reactive mounts.
fn shell_tree() -> UITree<Msg> {
    // The root's class needs a NodeRef, so build through a ContainerBuilder
    // and extract the single root node (same pattern as the results view).
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
        c.container(|header| {
            header.heading(2, "Cut List Optimizer");
            header
                .button("Help & examples")
                .class("cutlist-btn cutlist-ghost")
                .on_click(Msg::ToggleHelp);
        })
        .class("cutlist-header cutlist-no-print");
        c.text("Paste your parts, set the stock, optimize — everything runs locally in your browser.")
            .class("cutlist-sub")
            .class("cutlist-no-print");

        c.container(|card| {
            card.heading(3, "1 \u{b7} Parts")
                .class("cutlist-no-print");
            card.text("One part per line: length x quantity, optional label. Tags: @material, !grain. Comma-separated rows (CSV) also paste fine.")
                .class("cutlist-hint");
            card.textarea("")
                .class("cutlist-parts")
                .on_input(|value| Msg::PartsChanged(value));
            card.container(|_| {}).class("cutlist-parts-live");

            card.container(|examples| {
                for (index, (name, _, _, _)) in EXAMPLES.iter().enumerate() {
                    examples
                        .button(*name)
                        .class("cutlist-btn cutlist-ghost")
                        .on_click(Msg::LoadExample(index));
                }
            })
            .class("cutlist-examples");

            card.container(|_| {}).class("cutlist-divider");

            card.heading(3, "2 \u{b7} Stock & blade")
                .class("cutlist-no-print");
            card.container(|fields| {
                #[cfg(feature = "pro")]
                {
                    fields
                        .container(|field| {
                            field
                                .text("Stock lines: length [x max bars] [@material]")
                                .class("cutlist-label");
                            field
                                .textarea(sample_stock())
                                .class("cutlist-stocks")
                                .on_input(|value| Msg::StocksChanged(value));
                        })
                        .class("cutlist-field");
                }
                #[cfg(not(feature = "pro"))]
                {
                    fields
                        .container(|field| {
                            field.text("Stock length").class("cutlist-label");
                            field
                                .input(sample_stock())
                                .class("cutlist-input cutlist-stock");
                        })
                        .class("cutlist-field");
                }
                fields
                    .container(|field| {
                        field.text("Blade / kerf width").class("cutlist-label");
                        field
                            .input(sample_kerf())
                            .class("cutlist-input cutlist-kerf");
                    })
                    .class("cutlist-field");
                // The units selector mounts here (reactive).
                fields.container(|_| {}).class("cutlist-units-root");
            })
            .class("cutlist-row");
            #[cfg(feature = "pro")]
            card.container(|_| {}).class("cutlist-stocks-live");

            card.container(|actions| {
                actions
                    .button("Calibrate kerf\u{2026}")
                    .class("cutlist-btn cutlist-ghost")
                    .on_click(Msg::ToggleKerfWiz);
            })
            .class("cutlist-actions cutlist-no-print");
            card.container(|_| {}).class("cutlist-kerf-wiz-root cutlist-no-print");

            card.container(|_| {}).class("cutlist-divider");

            card.heading(3, "3 \u{b7} Optimize")
                .class("cutlist-no-print");
            card.container(|actions| {
                actions
                    .button("Optimize cuts")
                    .class("cutlist-btn cutlist-primary")
                    .on_click(Msg::Optimize);
                actions
                    .button("Clear")
                    .class("cutlist-btn")
                    .on_click(Msg::Clear);
                actions
                    .button("Import CSV")
                    .class("cutlist-btn")
                    .on_click(Msg::ImportCsv);
                actions
                    .button("Open job\u{2026}")
                    .class("cutlist-btn")
                    .on_click(Msg::OpenProject);
                actions
                    .button("Save job")
                    .class("cutlist-btn")
                    .on_click(Msg::SaveProject);
                #[cfg(feature = "pro")]
                actions
                    .button("Print layout")
                    .class("cutlist-btn")
                    .on_click(Msg::Print);
            })
            .class("cutlist-actions");
        })
        .class("cutlist-card cutlist-no-print");

        // The collapsible guide mounts into this placeholder (open by default).
        c.container(|_| {}).class("cutlist-help-root cutlist-no-print");

        // Pro: the reactive saved-stock-set library.
        #[cfg(feature = "pro")]
        c.container(|_| {}).class("cutlist-library");

        // The reactive results section mounts into this placeholder.
        c.container(|_| {}).class("cutlist-results cutlist-no-print");

        // Pro: the print-only shop-floor layout mounts here.
        #[cfg(feature = "pro")]
        c.container(|_| {}).class("cutlist-print-root");

        // Free edition: the Pro upsell mounts here, nudging only after a
        // few jobs have been optimized on this device.
        #[cfg(not(feature = "pro"))]
        c.container(|_| {}).class("cutlist-upsell-root");
        })
        // The app styles are scoped to `.cutlist-app`; apply it to our own
        // root so standalone (trunk/desktop) renders styled no matter what
        // class the host container carries. Harmless when the hub already
        // sets it.
        .class("cutlist-app");
    root_builder
        .into_only_child()
        .expect("shell builds exactly one root container")
}

/// Reads the input fields, validates, runs the solver, and publishes the
/// outcome. Runs inside the click handler (not inside an effect), so the
/// 200 ms Pro search is off the reactive graph.
fn run_optimize(
    container: &web_sys::Element,
    results: &Signal<Outcome>,
    units: &Signal<Units>,
    opt_count: &Signal<u32>,
) {
    let parts_text = read_textarea(container, ".cutlist-parts");
    let stock_text = read_stock_field(container);
    let kerf_text = read_input(container, ".cutlist-kerf");
    let units_now = units.get();

    let mut issues: Vec<Issue> = Vec::new();

    let kerf = match parse::parse_len(&kerf_text, units_now) {
        Some(value) if value <= 50 => value,
        _ => {
            issues.push(Issue {
                line: None,
                message: format!(
                    "Kerf width must be a length in {} between 0 and 50 mm.",
                    units_now.long_name()
                ),
            });
            0
        }
    };

    let parts = match parse::parse_parts(&parts_text, units_now) {
        Ok(parts) => parts,
        Err(line_issues) => {
            issues.extend(line_issues);
            Vec::new()
        }
    };

    // Edition-specific stock collection with inline validation.
    #[cfg(feature = "pro")]
    let (stocks, material_names) = collect_pro_stocks(&stock_text, &parts, &mut issues, units_now);
    #[cfg(not(feature = "pro"))]
    let (stocks, material_names) = collect_free_stock(&stock_text, &parts, &mut issues, units_now);

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
        Ok(solution) => {
            // Remember the job on this device so a relaunch continues where
            // the user left off.
            storage_set("cutlist-last-parts", &parts_text);
            storage_set("cutlist-last-stock", &stock_text);
            storage_set("cutlist-last-kerf", &kerf_text);
            // Upgrade-funnel metrics: count optimizations per device.
            let count = opt_count.get().saturating_add(1);
            storage_set("cutlist-optimize-count", &count.to_string());
            opt_count.set(count);
            // Keep the shareable permalink in the URL in sync with the job.
            let encoded = encode_job(&parts_text, &stock_text, &kerf_text, units_now.name());
            update_hash(&encoded);
            results.set(Outcome::Solved {
                solution,
                parts,
                stocks,
                material_names,
                kerf,
            });
        }
        Err(error) => results.set(Outcome::Invalid(vec![Issue {
            line: None,
            message: error.to_string(),
        }])),
    }
}

/// Reads whichever field holds the stock definition in this edition
/// (single length input on free, multi-line textarea on Pro).
fn read_stock_field(container: &web_sys::Element) -> String {
    #[cfg(feature = "pro")]
    {
        read_textarea(container, ".cutlist-stocks")
    }
    #[cfg(not(feature = "pro"))]
    {
        read_input(container, ".cutlist-stock")
    }
}

/// Free edition: a single stock length field, all parts of one material.
#[cfg(not(feature = "pro"))]
fn collect_free_stock(
    stock_text: &str,
    parts: &[ParsedPart],
    issues: &mut Vec<Issue>,
    units: Units,
) -> (Vec<StockSpec>, Vec<String>) {
    let stock_length = match parse::parse_len(stock_text, units) {
        Some(value) if value > 0 => value,
        _ => {
            issues.push(Issue {
                line: None,
                message: format!(
                    "Stock length must be a length in {} (e.g. 2400).",
                    units.long_name()
                ),
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
    stocks_text: &str,
    parts: &[ParsedPart],
    issues: &mut Vec<Issue>,
    units: Units,
) -> (Vec<StockSpec>, Vec<String>) {
    let parsed_stocks = match parse::parse_stocks(stocks_text, units) {
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

/// Converts a parts textarea (canonical millimetre text, e.g. an example
/// job) into the target unit. Returns the input unchanged if it is already
/// in millimetres or does not parse.
fn convert_parts_text(text_mm: &str, from: Units, to: Units) -> String {
    if from == to {
        return text_mm.to_string();
    }
    parse::parse_parts(text_mm, from)
        .map(|parts| parts_to_lines(&parts, to))
        .unwrap_or_else(|_| text_mm.to_string())
}

/// Converts a Pro stock textarea between units (same rules as parts).
#[cfg(feature = "pro")]
fn convert_stocks_text(text_mm: &str, from: Units, to: Units) -> String {
    if from == to {
        return text_mm.to_string();
    }
    parse::parse_stocks(text_mm, from)
        .map(|stocks| stocks_to_lines(&stocks, to))
        .unwrap_or_else(|_| text_mm.to_string())
}

/// Converts a single number (free stock length, kerf) between units.
fn convert_single_text(text_mm: &str, from: Units, to: Units) -> String {
    if from == to {
        return text_mm.to_string();
    }
    match parse::parse_len(text_mm, from) {
        Some(mm) => fmt_len_plain(mm, to),
        None => text_mm.to_string(),
    }
}

/// Converts a DOM field's contents between units in place, keeping the
/// paired counter signal in sync.
fn convert_dom_field(
    container: &web_sys::Element,
    selector: &str,
    from: Units,
    to: Units,
    signal: &Signal<String>,
) {
    let text = read_textarea(container, selector);
    if text.trim().is_empty() {
        return;
    }
    let converted = convert_parts_text(&text, from, to);
    write_field(container, selector, &converted);
    signal.set(converted);
}

/// Builds the results section tree. The child shape is invariant —
/// [summary, stats, errors, bars wrapper, sequence wrapper, inventory
/// wrapper, data grid, actions] at fixed positions with fixed node kinds —
/// so the positional reconciler can always update in place: a position
/// whose kind flips (e.g. a div appearing where the Idle tree had the
/// grid's `table`) aborts reconciliation mid-render. Variable content lives
/// INSIDE the wrappers (same-kind children only).
fn results_view(
    results: Signal<Outcome>,
    units: Signal<Units>,
    offcut_min: Signal<u32>,
) -> UITree<Msg> {
    let outcome = results.get();
    let units_now = units.get();
    let min_offcut = offcut_min.get();

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
                    "Pick an example above, or paste your parts and press Optimize cuts."
                        .to_string(),
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
                        "{} bar{} used \u{b7} {} of stock \u{b7} {:.1}% utilization",
                        solution.bar_count(),
                        if solution.bar_count() == 1 { "" } else { "s" },
                        fmt_len_u64(stock_total, units_now),
                        solution.utilization * 100.0
                    );
                    #[cfg(feature = "pro")]
                    let headline =
                        format!("{headline} \u{b7} cost {}", grouped(solution.cost));
                    (
                        headline,
                        format!(
                            "Requested {} of parts \u{b7} {} total offcut (kerf {} between cuts)",
                            fmt_len_u64(solution.parts_length() as u64, units_now),
                            fmt_len_u64(solution.waste as u64, units_now),
                            fmt_len(*kerf, units_now)
                        ),
                    )
                }
            };
            summary.text(headline).class("cutlist-headline");
            summary.text(detail).class("cutlist-detail");
            summary.text(ENGINE_NOTE).class("cutlist-engine");
        })
        .class("cutlist-summary");

        // Stat cards — always four, so the shape stays stable for the
        // in-place reconciler across Idle / Invalid / Solved.
        c.container(|stats| {
            #[allow(unused_variables)]
            let (bars, utilization, offcut, fourth_value, fourth_label) = match &outcome {
                Outcome::Solved {
                    solution,
                    stocks,
                    ..
                } => {
                    let stock_total: u64 = solution
                        .bars
                        .iter()
                        .map(|bar| u64::from(stocks[bar.stock_index].length))
                        .sum();
                    #[cfg(feature = "pro")]
                    let (fourth_value, fourth_label) =
                        (grouped(solution.cost), "Stock cost".to_string());
                    #[cfg(not(feature = "pro"))]
                    let (fourth_value, fourth_label) = (
                        fmt_len_u64(stock_total, units_now),
                        "Stock used".to_string(),
                    );
                    (
                        solution.bar_count().to_string(),
                        format!("{:.1}%", solution.utilization * 100.0),
                        fmt_len_u64(solution.waste as u64, units_now),
                        fourth_value,
                        fourth_label,
                    )
                }
                _ => (
                    "\u{2014}".to_string(),
                    "\u{2014}".to_string(),
                    "\u{2014}".to_string(),
                    "\u{2014}".to_string(),
                    "Stock".to_string(),
                ),
            };
            for (label, value) in [
                ("Bars used", bars),
                ("Utilization", utilization),
                ("Total offcut", offcut),
                (fourth_label.as_str(), fourth_value),
            ] {
                stats.container(|stat| {
                    stat.text(value).class("cutlist-stat-value");
                    stat.text(label).class("cutlist-stat-label");
                })
                .class("cutlist-stat");
            }
        })
        .class("cutlist-stats");

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

        // Bars live in an always-present wrapper so the wrapper positions
        // after it (sequence, inventory, grid) never change kind.
        c.container(|bars| append_bars(bars, &outcome, units_now))
            .class("cutlist-bars");

        // Cutting sequence: the ordered saw-ops list, one line per bar.
        // Lines are wrapped in divs — the DOM backend drops classes on bare
        // text nodes, and the div keeps the row styling + kind stability.
        c.container(|sequence| {
            if matches!(outcome, Outcome::Solved { .. }) {
                sequence.heading(3, "Cutting sequence").class("cutlist-seq-head");
                for line in sequence_lines(&outcome, units_now) {
                    sequence
                        .container(|row| {
                            row.text(line);
                        })
                        .class("cutlist-seq-line");
                }
            }
        })
        .class("cutlist-sequence");

        // Offcut inventory: every leftover over the bank threshold.
        c.container(|inventory| {
            if matches!(outcome, Outcome::Solved { .. }) {
                inventory
                    .heading(3, "Offcut inventory")
                    .class("cutlist-seq-head");
                for line in inventory_lines(&outcome, min_offcut, units_now) {
                    inventory
                        .container(|row| {
                            row.text(line);
                        })
                        .class("cutlist-seq-line");
                }
            }
        })
        .class("cutlist-inventory");

        c.data_grid(grid_columns(), placement_rows(&outcome, units_now));

        c.container(|row| {
            row.button("Download CSV")
                .class("cutlist-btn")
                .on_click(Msg::ExportCsv);
            row.button("Download SVG")
                .class("cutlist-btn")
                .on_click(Msg::ExportSvg);
            row.button("Copy share link")
                .class("cutlist-btn cutlist-share")
                .on_click(Msg::CopyLink);
            #[cfg(feature = "pro")]
            {
                row.input(&min_offcut.to_string())
                    .class("cutlist-input cutlist-thresh")
                    .on_input(|value| Msg::OffcutMinChanged(value));
                row.button("Save offcuts as stock")
                    .class("cutlist-btn")
                    .on_click(Msg::SaveOffcuts);
                row.text(format!(
                    "Leftovers of {} or more are appended to your stock lines.",
                    fmt_len(min_offcut, units_now)
                ))
                .class("cutlist-hint");
            }
        })
        .class("cutlist-actions cutlist-no-print");
        })
        .class("cutlist-results-view");
    root_builder
        .into_only_child()
        .expect("results view builds exactly one root container")
}

/// Appends one proportional cut diagram per bar (children of the bars
/// container — all containers, so reconciliation stays in-place).
#[cfg_attr(not(feature = "pro"), allow(unused_variables))]
fn append_bars(
    c: &mut tpt_appfront_core::ContainerBuilder<Msg>,
    outcome: &Outcome,
    units: Units,
) {
    let Outcome::Solved {
        solution,
        parts,
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
        let waste = bar.waste(stock_length, *kerf, 0) as u64;

        let header = bar_header_text(bar_index, bar, stocks, material_names, *kerf, units);

        c.container(|bar_node| {
            bar_node.text(header).class("cutlist-bar-head");
            bar_node.container(|track| {
                for cut in &bar.cuts {
                    let tenths = cut_tenths(cut.length, stock_length);
                    let color = cut.part % 8;
                    let label = display_label(&parts[cut.part].label, cut.part);
                    let mut title = format!(
                        "{label} \u{b7} {} at {}",
                        fmt_len(cut.length, units),
                        fmt_len(cut.offset, units)
                    );
                    if parts[cut.part].grain {
                        title.push_str(" \u{b7} grain along length");
                    }
                    #[cfg(feature = "pro")]
                    {
                        let material = stocks[bar.stock_index].material;
                        if let Some(name) =
                            material_names.get(material).filter(|n| !n.is_empty())
                        {
                            title.push_str(&format!(" \u{b7} {name}"));
                        }
                    }
                    track
                        .container(|_| {})
                        .class(format!("cutlist-seg cutlist-c{color} cutlist-w-{tenths}"))
                        .attr("title", title);
                }
                let spent: u32 = bar
                    .cuts
                    .iter()
                    .map(|cut| u32::from(cut_tenths(cut.length, stock_length)))
                    .sum();
                let waste_tenths = 1000u32.saturating_sub(spent.min(1000));
                track
                    .container(|_| {})
                    .class(format!("cutlist-waste cutlist-w-{waste_tenths}"))
                    .attr(
                        "title",
                        format!("Offcut \u{b7} {}", fmt_len(waste as u32, units)),
                    );
            })
            .class("cutlist-track");

            // Legend: color swatch -> part name (and how many of it are on
            // this bar), matching the segment colors above.
            bar_node.container(|legend| {
                let mut counts: BTreeMap<usize, u32> = BTreeMap::new();
                for cut in &bar.cuts {
                    *counts.entry(cut.part).or_insert(0) += 1;
                }
                for (part_index, count) in counts {
                    let color = part_index % 8;
                    legend
                        .container(|_| {})
                        .class(format!("cutlist-swatch cutlist-c{color}"));
                    let mut name = display_label(&parts[part_index].label, part_index);
                    if parts[part_index].grain {
                        name.push_str(" [grain]");
                    }
                    if count > 1 {
                        legend
                            .text(format!("{name} \u{d7}{count}"))
                            .class("cutlist-legend-label");
                    } else {
                        legend.text(name).class("cutlist-legend-label");
                    }
                }
            })
            .class("cutlist-legend");
        })
        .class("cutlist-bar");
    }
}

/// The per-bar diagram header, shared by the on-screen and SVG renderers.
#[cfg_attr(not(feature = "pro"), allow(unused_variables))]
fn bar_header_text(
    bar_index: usize,
    bar: &tpt_cutlist_solver::Bar,
    stocks: &[StockSpec],
    material_names: &[String],
    kerf: u32,
    units: Units,
) -> String {
    let stock = &stocks[bar.stock_index];
    let used = bar.parts_length() as u64 + bar.kerf_length(kerf) as u64;
    let waste = bar.waste(stock.length, kerf, 0) as u64;
    let header = format!(
        "Bar {} \u{b7} {} cuts \u{b7} {} / {} used \u{b7} {} offcut",
        bar_index + 1,
        bar.cut_count(),
        fmt_len_u64(used, units),
        fmt_len(stock.length, units),
        fmt_len_u64(waste, units),
    );
    #[cfg(feature = "pro")]
    let header = match material_names.get(stock.material).filter(|n| !n.is_empty()) {
        Some(name) => format!("{header} \u{b7} {name}"),
        None => header,
    };
    header
}

/// One row per placed cut for the placement table.
#[cfg_attr(not(feature = "pro"), allow(unused_variables))]
fn placement_rows(outcome: &Outcome, units: Units) -> Vec<Vec<String>> {
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
                fmt_len(cut.length, units),
                fmt_len(cut.offset, units),
                if parts[cut.part].grain {
                    "grain".to_string()
                } else {
                    String::new()
                },
                format!("{}", bar_index + 1),
            ];
            #[cfg(feature = "pro")]
            {
                let material = stocks[bar.stock_index].material;
                let name = material_names
                    .get(material)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| "\u{2014}".to_string());
                row.insert(1, name);
            }
            rows.push(row);
        }
    }
    rows
}

/// Placement-table columns for this edition (Pro adds Material).
fn grid_columns() -> Vec<&'static str> {
    #[cfg(feature = "pro")]
    {
        vec!["Part", "Material", "Length", "Position", "Grain", "Bar"]
    }
    #[cfg(not(feature = "pro"))]
    {
        vec!["Part", "Length", "Position", "Grain", "Bar"]
    }
}

/// Ordered saw-ops list, one line per bar: which piece to cut, how long,
/// and where the saw lands (offsets already include kerf).
fn sequence_lines(outcome: &Outcome, units: Units) -> Vec<String> {
    let Outcome::Solved {
        solution,
        parts,
        stocks,
        kerf,
        ..
    } = outcome
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for (bar_index, bar) in solution.bars.iter().enumerate() {
        let ops: Vec<String> = bar
            .cuts
            .iter()
            .map(|cut| {
                let label = display_label(&parts[cut.part].label, cut.part);
                format!(
                    "{label} ({}) at {}",
                    fmt_len(cut.length, units),
                    fmt_len(cut.offset, units)
                )
            })
            .collect();
        let waste = bar.waste(stocks[bar.stock_index].length, *kerf, 0);
        lines.push(format!(
            "Bar {}: {} \u{2192} offcut {}",
            bar_index + 1,
            ops.join(", then "),
            fmt_len(waste, units)
        ));
    }
    lines
}

/// Offcut inventory: every bar leftover of at least `min` millimetres,
/// grouped by length + material.
fn inventory_lines(outcome: &Outcome, min: u32, units: Units) -> Vec<String> {
    let Outcome::Solved {
        solution,
        stocks,
        material_names,
        kerf,
        ..
    } = outcome
    else {
        return Vec::new();
    };
    let mut counts: BTreeMap<(u32, String), u32> = BTreeMap::new();
    for bar in &solution.bars {
        let stock = &stocks[bar.stock_index];
        let waste = bar.waste(stock.length, *kerf, 0);
        if waste < min {
            continue;
        }
        let material = material_names
            .get(stock.material)
            .cloned()
            .unwrap_or_default();
        *counts.entry((waste, material)).or_insert(0) += 1;
    }
    if counts.is_empty() {
        return vec![format!(
            "Nothing at or over {} yet \u{2014} every bar is used to within the threshold.",
            fmt_len(min, units)
        )];
    }
    counts
        .into_iter()
        .map(|((length, material), count)| {
            let material_suffix = if material.is_empty() {
                String::new()
            } else {
                format!(" @{material}")
            };
            format!("{count} \u{d7} {}{}", fmt_len(length, units), material_suffix)
        })
        .collect()
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

/// Formats a length that exceeds u32 (stock totals) in the given unit.
fn fmt_len_u64(mm: u64, units: Units) -> String {
    match units {
        Units::Millimetres => format!("{} mm", grouped(mm)),
        Units::Inches => format!("{:.2} in", mm as f64 / parse::MM_PER_INCH),
    }
}

/// Live summary under the parts textarea: sizes, pieces and total length,
/// recomputed on every keystroke. Empty until there is something to show.
fn parts_live_view(parts_text: Signal<String>, units: Signal<Units>) -> UITree<Msg> {
    let text = parts_text.get();
    let units_now = units.get();
    let summary = match parse::parse_parts(&text, units_now) {
        Ok(parts) if !parts.is_empty() => {
            let pieces: u32 = parts.iter().map(|p| p.qty).sum();
            let total_mm: u64 = parts
                .iter()
                .map(|p| u64::from(p.length) * u64::from(p.qty))
                .sum();
            format!(
                "{} size{} \u{b7} {} piece{} \u{b7} {} to cut",
                parts.len(),
                plural(parts.len()),
                pieces,
                plural(pieces as usize),
                fmt_len_u64(total_mm, units_now)
            )
        }
        _ => String::new(),
    };

    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder.text(summary).class("cutlist-live");
    root_builder
        .into_only_child()
        .expect("live view builds exactly one root container")
}

/// Pro: live summary under the stock textarea.
#[cfg(feature = "pro")]
fn stocks_live_view(stocks_text: Signal<String>, units: Signal<Units>) -> UITree<Msg> {
    let text = stocks_text.get();
    let units_now = units.get();
    let summary = match parse::parse_stocks(&text, units_now) {
        Ok(stocks) if !stocks.is_empty() => {
            let materials: Vec<String> = {
                let mut names: Vec<String> = stocks
                    .iter()
                    .map(|s| {
                        if s.material.is_empty() {
                            "default".to_string()
                        } else {
                            s.material.clone()
                        }
                    })
                    .collect();
                names.sort();
                names.dedup();
                names
            };
            let capped: u32 = stocks
                .iter()
                .filter_map(|s| s.max_bars)
                .sum();
            format!(
                "{} stock size{} \u{b7} materials: {}{}",
                stocks.len(),
                plural(stocks.len()),
                materials.join(", "),
                if capped > 0 {
                    format!(" \u{b7} up to {capped} capped bars")
                } else {
                    String::new()
                }
            )
        }
        _ => String::new(),
    };

    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder.text(summary).class("cutlist-live");
    root_builder
        .into_only_child()
        .expect("live view builds exactly one root container")
}

/// The units selector — its own reactive mount so a device's remembered
/// unit shows correctly on load.
fn units_view(units: Signal<Units>) -> UITree<Msg> {
    let units_now = units.get();
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder.container(|field| {
        field.text("Units").class("cutlist-label");
        field
            .select(
                [("mm", "Millimetres"), ("in", "Inches")],
                units_now.name(),
            )
            .class("cutlist-input")
            .on_input(|value| Msg::SetUnits(value));
    })
    .class("cutlist-field");
    root_builder
        .into_only_child()
        .expect("units view builds exactly one root container")
}

/// The kerf calibration wizard ("cut twice, measure the gap"): the user
/// cuts a piece off a board and measures board / piece / remainder; the
/// blade kerf is the difference.
fn kerf_wiz_view(state: Signal<KerfWiz>, units: Signal<Units>) -> UITree<Msg> {
    let KerfWiz {
        open,
        before,
        piece,
        offcut,
        message,
        kerf_mm,
    } = state.get();
    let units_now = units.get();
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            if open {
                c.heading(3, "Calibrate the blade kerf");
                c.text(format!(
                    "Cut one piece off a board, then measure all three: the board before the cut, \
                     the piece you cut off, and what is left. Enter them in {}. The kerf is \
                     board \u{2212} piece \u{2212} remainder.",
                    units_now.long_name()
                ))
                .class("cutlist-hint");
                c.container(|row| {
                    for (label, class, value) in [
                        ("Board before cut", "cutwiz-before", before.as_str()),
                        ("Piece cut off", "cutwiz-piece", piece.as_str()),
                        ("Remainder", "cutwiz-offcut", offcut.as_str()),
                    ] {
                        row.container(|field| {
                            field.text(label).class("cutlist-label");
                            field
                                .input(value)
                                .class(format!("cutlist-input {class}"));
                        })
                        .class("cutlist-field");
                    }
                })
                .class("cutlist-row");
                c.container(|actions| {
                    actions
                        .button("Calculate kerf")
                        .class("cutlist-btn")
                        .on_click(Msg::CalcKerf);
                    if kerf_mm.is_some() {
                        actions
                            .button("Use this kerf")
                            .class("cutlist-btn cutlist-primary")
                            .on_click(Msg::UseKerf);
                    }
                })
                .class("cutlist-actions");
                c.text(&message).class("cutlist-live");
            }
        })
        .class(if open {
            "cutlist-card cutlist-no-print"
        } else {
            "cutlist-no-print"
        });
    root_builder
        .into_only_child()
        .expect("kerf wizard view builds exactly one root container")
}

/// Reads the wizard's three measurements and solves for the kerf.
fn calculate_kerf(
    container: &web_sys::Element,
    kerf_wiz: &Signal<KerfWiz>,
    units: &Signal<Units>,
) {
    let units_now = units.get();
    let before = read_input(container, ".cutwiz-before");
    let piece = read_input(container, ".cutwiz-piece");
    let offcut = read_input(container, ".cutwiz-offcut");

    let (message, kerf_mm) =
        match (
            parse::parse_len(&before, units_now),
            parse::parse_len(&piece, units_now),
            parse::parse_len(&offcut, units_now),
        ) {
            (Some(b), Some(p), Some(o)) if p + o > b => (
                "The piece plus the remainder is longer than the board \u{2014} re-check the \
                 measurements."
                    .to_string(),
                None,
            ),
            (Some(b), Some(p), Some(o)) => {
                let kerf = b - p - o;
                if kerf > 50 {
                    (
                        "That kerf is over 50 mm \u{2014} check the measurements.".to_string(),
                        None,
                    )
                } else {
                    (
                        format!("Blade kerf \u{2248} {} \u{2014} press \u{201c}Use this kerf\u{201d}.", fmt_len(kerf, units_now)),
                        Some(kerf),
                    )
                }
            }
            _ => ("Enter all three measurements as numbers.".to_string(), None),
        };

    kerf_wiz.set(KerfWiz {
        open: true,
        before,
        piece,
        offcut,
        message,
        kerf_mm,
    });
}

/// The collapsible "How it works" guide — open by default so first-time
/// users see the format, what kerf means, and what they get.
#[cfg_attr(feature = "pro", allow(unused_variables))]
fn help_view(open: Signal<bool>) -> UITree<Msg> {
    let open = open.get();
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            c.heading(3, "How it works");
            c.text(
                "1 \u{b7} Parts — one piece type per line: `length x quantity label`. \
                 `600 x 3 Shelf` means three 600 shelves. Past CSV rows (`600, 3, Shelf`) \
                 parse fine, `#` lines are comments, and duplicate lines merge their \
                 quantities. Add `!grain` to note that grain runs along the part \
                 (shown in the table and print sheet).",
            )
            .class("cutlist-help-p");
            #[cfg(feature = "pro")]
            c.text(
                "Materials — tag a part with @name (`600 x 2 Shelf @ply`) and add one stock \
                 line per stock: `2400 x 4 @ply` means 2400 stock, at most 4 bars. A part \
                 is only ever cut from its own material, and pieces that fit no stock of \
                 that material are flagged before optimizing.",
            )
            .class("cutlist-help-p");
            c.text(
                "2 \u{b7} Stock & blade — the stock length is the board you cut from. Kerf is \
                 the blade width lost to every cut (about 3 for a circular saw, 4\u{2013}5 \
                 for a track saw) — use \u{201c}Calibrate kerf\u{201d} to measure yours. The \
                 Units selector switches everything between millimetres and inches. The \
                 optimizer charges kerf between cuts, so the packing is real-world accurate.",
            )
            .class("cutlist-help-p");
            c.text(
                "3 \u{b7} Optimize — every piece is packed onto bars, drawn to scale, with exact \
                 cut positions in the table, an ordered cutting sequence per bar, and an \
                 offcut inventory. \u{201c}Copy share link\u{201d} puts the whole job in the URL, \
                 so a link opens on someone else's device already filled in and solved. \
                 \u{201c}Save job\u{201d} writes a .cutlist.json file you can reopen (or share); \
                 \u{201c}Import CSV\u{201d} reads length,qty,label spreadsheets.",
            )
            .class("cutlist-help-p");
            #[cfg(feature = "pro")]
            c.text(
                "Printing — the Print layout button opens a shop-floor sheet with the diagrams, \
                 cutting sequence and cut positions, ready to take to the saw. Saved stock sets \
                 let you store your stock lengths once and reload them any time (kept on this \
                 device), and \u{201c}Save offcuts as stock\u{201d} banks leftovers over the \
                 threshold for the next job.",
            )
            .class("cutlist-help-p");
            c.text(
                "Everything runs locally — your cut list never leaves this device.",
            )
            .class("cutlist-help-p");
            c.text(format!(
                "Cut List Optimizer v{} \u{b7} free edition: one stock length, fast packing. \
                 Pro adds multiple materials, the improved search, saved stock sets and printing.",
                env!("CARGO_PKG_VERSION")
            ))
            .class("cutlist-engine");
        })
        .class(if open {
            "cutlist-help cutlist-open"
        } else {
            "cutlist-help"
        });
    root_builder
        .into_only_child()
        .expect("help view builds exactly one root container")
}

/// Free edition: the Pro upsell — hidden until this device has run a few
/// optimizations, then shown as a softer nudge than an immediate ad.
#[cfg(not(feature = "pro"))]
fn upsell_view(opt_count: Signal<u32>) -> UITree<Msg> {
    let show = opt_count.get() >= NUDGE_AFTER;
    let mut root_builder = tpt_appfront_core::ContainerBuilder::<Msg>::new();
    root_builder
        .container(|c| {
            if show {
                c.heading(3, "Need more than one stock length?");
                c.text(
                    "The Pro edition ($49) adds multi-stock and multi-material jobs, the improved \
                     optimization search, saved stock-set libraries, and print-ready shop-floor \
                     layout export.",
                );
                c.link(
                    "https://tptsolutions.co.nz/tools/cutlist-optimizer",
                    "Get the Pro edition",
                );
            }
        })
        .class(if show {
            "cutlist-upsell cutlist-no-print"
        } else {
            "cutlist-upsell-hidden cutlist-no-print"
        });
    root_builder
        .into_only_child()
        .expect("upsell view builds exactly one root container")
}

/// Pro: the print-only shop-floor layout. Lives in its own reactive mount
/// (hidden on screen via `.cutlist-print`, revealed by the print stylesheet
/// when the "Print layout" button calls `window.print()`). Mirrors the
/// results section so it is always in sync.
#[cfg(feature = "pro")]
fn print_view(results: Signal<Outcome>, units: Signal<Units>) -> UITree<Msg> {
    let outcome = results.get();
    let units_now = units.get();

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
            // Fixed child kinds — [h2, text, text, bars, sequence, grid] —
            // in both the idle and solved states (see results_view).
            if let Outcome::Solved {
                solution,
                stocks,
                kerf,
                ..
            } = &outcome
            {
                let stock_total: u64 = solution
                    .bars
                    .iter()
                    .map(|bar| u64::from(stocks[bar.stock_index].length))
                    .sum();
                c.text(format!(
                    "{} bar{} \u{b7} {} stock \u{b7} {} parts \u{b7} {} offcut \u{b7} kerf {}",
                    solution.bar_count(),
                    if solution.bar_count() == 1 { "" } else { "s" },
                    fmt_len_u64(stock_total, units_now),
                    fmt_len_u64(solution.parts_length() as u64, units_now),
                    fmt_len_u64(solution.waste as u64, units_now),
                    fmt_len(*kerf, units_now)
                ))
                .class("cutlist-detail");
                c.text(ENGINE_NOTE).class("cutlist-engine");
            } else {
                c.text("Optimize a job first — the print layout mirrors the results.")
                    .class("cutlist-engine");
                c.text("");
            }

            c.container(|bars| append_bars(bars, &outcome, units_now))
                .class("cutlist-bars");
            c.container(|sequence| {
                if matches!(outcome, Outcome::Solved { .. }) {
                    sequence.heading(3, "Cutting sequence").class("cutlist-seq-head");
                    for line in sequence_lines(&outcome, units_now) {
                        sequence
                            .container(|row| {
                                row.text(line);
                            })
                            .class("cutlist-seq-line");
                    }
                }
            })
            .class("cutlist-sequence");

            c.data_grid(grid_columns(), placement_rows(&outcome, units_now));
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
            c.heading(3, "Saved stock sets");
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

/// Pro: the offcut bank — every leftover of at least the threshold becomes
/// a stock line (`1234 x 1 @material`) appended to the stock textarea, so
/// the next job reuses real offcuts instead of fresh stock.
#[cfg(feature = "pro")]
fn save_offcuts(
    container: &web_sys::Element,
    results: &Signal<Outcome>,
    stocks_text: &Signal<String>,
    offcut_min: &Signal<u32>,
) {
    let min_offcut = offcut_min.get();
    let outcome = results.get();
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

    let mut lines: Vec<String> = Vec::new();
    for bar in &solution.bars {
        let stock = &stocks[bar.stock_index];
        let waste = bar.waste(stock.length, kerf, 0);
        if waste < min_offcut {
            continue;
        }
        match material_names.get(stock.material).filter(|n| !n.is_empty()) {
            Some(name) => lines.push(format!("{waste} x 1 @{name}")),
            None => lines.push(format!("{waste} x 1")),
        }
    }
    if lines.is_empty() {
        return;
    }

    let mut text = read_stock_field(container).trim_end().to_string();
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&lines.join("\n"));
    write_field(container, ".cutlist-stocks", &text);
    stocks_text.set(text);
}

/// Reads a `#/j=<payload>` permalink from the location bar, if present.
fn read_permalink_job() -> Option<(String, String, String, String)> {
    let window = web_sys::window()?;
    let hash = window.location().hash().ok()?;
    let payload = hash.strip_prefix("#/j=")?;
    decode_job(payload)
}

/// Rewrites the URL hash in place (no history entry) to carry the current
/// job, or strips it when the payload is too large for a link.
fn update_hash(encoded: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(history) = window.history() else {
        return;
    };
    if encoded.len() <= MAX_HASH_LEN {
        let _ = history.replace_state_with_url(
            &JsValue::NULL,
            "",
            Some(&format!("#/j={encoded}")),
        );
    } else {
        clear_hash();
    }
}

/// Strips the hash from the URL without adding a history entry.
fn clear_hash() {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let (Ok(history), Ok(href)) = (window.history(), window.location().href()) {
        let base = href.split('#').next().unwrap_or("");
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(base));
    }
}

/// Free-edition "Copy share link": encodes the job into the URL hash and
/// copies the full URL, so anyone opening it lands on the solved job.
fn copy_share_link(container: &web_sys::Element, units: &Signal<Units>) {
    let parts = read_textarea(container, ".cutlist-parts");
    let stock = read_stock_field(container);
    let kerf = read_input(container, ".cutlist-kerf");
    let encoded = encode_job(&parts, &stock, &kerf, units.get().name());
    update_hash(&encoded);
    let url = web_sys::window()
        .and_then(|window| window.location().href().ok())
        .unwrap_or_default();
    copy_to_clipboard(&url);
    flash_button(container, ".cutlist-share", "Link copied \u{2713}", "Copy share link");
}

/// Best-effort clipboard write: async Clipboard API first (secure
/// contexts), then the deprecated `execCommand('copy')` fallback.
fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    // Reflect-based access: `navigator.clipboard` is undefined in insecure
    // contexts, and a typed call on undefined would trap the wasm module.
    let via_api = (|| -> Option<()> {
        let navigator = window.navigator();
        let clipboard = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard")).ok()?;
        if clipboard.is_undefined() {
            return None;
        }
        let write_text = js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText")).ok()?;
        let write_text: js_sys::Function = write_text.dyn_into().ok()?;
        let _ = write_text.call1(&clipboard, &JsValue::from_str(text));
        Some(())
    })()
    .is_some();
    if via_api {
        return;
    }
    if let Some(document) = window.document() {
        if let Ok(area) = document.create_element("textarea") {
            area.set_text_content(Some(text));
            let _ = area.set_attribute("style", "position:fixed;opacity:0");
            if let Some(body) = document.body() {
                let _ = body.append_child(&area);
                if let Some(html_doc) = document.dyn_ref::<web_sys::HtmlDocument>() {
                    let _ = html_doc.exec_command("copy");
                }
                let _ = body.remove_child(&area);
            }
        }
    }
}

/// Brief button feedback ("Link copied ✓") that reverts after ~1.6 s.
fn flash_button(container: &web_sys::Element, selector: &str, message: &str, restore: &str) {
    let Some(button) = container.query_selector(selector).ok().flatten() else {
        return;
    };
    button.set_text_content(Some(message));
    let button_for_timer = button.clone();
    let restore = restore.to_string();
    let timer = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        button_for_timer.set_text_content(Some(&restore));
    });
    if let Some(window) = web_sys::window() {
        let callback: &js_sys::Function = timer.as_ref().unchecked_ref();
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback, 1600);
    }
    timer.forget();
}

/// Opens a native file picker and hands the chosen file's text to
/// `on_text`. The hidden input is unmounted immediately after the click —
/// its `change` listener keeps working detached, and leaking the closures
/// matches the app's page-lifetime style.
fn pick_text_file(accept: &str, on_text: Rc<dyn Fn(String)>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(input) = document.create_element("input") else {
        return;
    };
    let _ = input.set_attribute("type", "file");
    let _ = input.set_attribute("accept", accept);
    let _ = input.set_attribute("style", "display:none");
    let input_el: web_sys::HtmlInputElement = input.clone().dyn_into().expect("file input");
    let on_change = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |event: web_sys::Event| {
            let target = event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            let Some(file) = target
                .as_ref()
                .and_then(|i| i.files())
                .and_then(|files| files.item(0))
            else {
                return;
            };
            let blob: &web_sys::Blob = file.unchecked_ref();
            let on_text = on_text.clone();
            let on_loaded = wasm_bindgen::closure::Closure::<dyn FnMut(JsValue)>::new(
                move |value: JsValue| {
                    on_text(value.as_string().unwrap_or_default());
                },
            );
            // `Promise::then` via Reflect: the typed js-sys signature has
            // churned across versions (ScopedClosure), this call never does.
            let promise = blob.text();
            if let Some(then_fn) = js_sys::Reflect::get(&promise, &JsValue::from_str("then"))
                .ok()
                .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
            {
                let _ = then_fn.call1(&promise, on_loaded.as_ref());
            }
            on_loaded.forget();
            // Allow picking the same file again later.
            input_el.set_value("");
        },
    );
    let _ = input.add_event_listener_with_callback(
        "change",
        on_change.as_ref().unchecked_ref(),
    );
    on_change.forget();
    if let Some(body) = document.body() {
        let _ = body.append_child(&input);
    }
    if let Some(el) = input.dyn_ref::<web_sys::HtmlInputElement>() {
        el.click();
    }
    if let Some(body) = document.body() {
        let _ = body.remove_child(&input);
    }
}

/// CSV import: reads a `length,qty,label[,material]` file in the current
/// unit, fills the parts box, and optimizes.
fn import_csv(
    container: &web_sys::Element,
    results: &Signal<Outcome>,
    units: &Signal<Units>,
    parts_text: &Signal<String>,
    opt_count: &Signal<u32>,
) {
    let container = container.clone();
    let results = results.clone();
    let units = units.clone();
    let parts_text = parts_text.clone();
    let opt_count = opt_count.clone();
    pick_text_file(
        ".csv,text/csv,text/comma-separated-values,text/plain",
        Rc::new(move |text| {
            let materials = cfg!(feature = "pro");
            match csv_to_parts_lines(&text, units.get(), materials) {
                Ok(lines) => {
                    write_field(&container, ".cutlist-parts", &lines);
                    parts_text.set(lines);
                    run_optimize(&container, &results, &units, &opt_count);
                }
                Err(issues) => results.set(Outcome::Invalid(issues)),
            }
        }),
    );
}

/// Project files: writes the whole job as `.cutlist.json` to downloads.
fn save_project(container: &web_sys::Element, units: &Signal<Units>) {
    let job = ProjectJob {
        units: units.get().name().to_string(),
        parts: read_textarea(container, ".cutlist-parts"),
        stocks: read_stock_field(container),
        kerf: read_input(container, ".cutlist-kerf"),
    };
    if job.parts.trim().is_empty() {
        return;
    }
    download_data_uri(
        "job.cutlist.json",
        "application/json",
        &project::write_project_json(&job),
    );
}

/// Project files: opens a `.cutlist.json`, fills every field, and solves.
fn open_project(
    container: &web_sys::Element,
    results: &Signal<Outcome>,
    units: &Signal<Units>,
    parts_text: &Signal<String>,
    opt_count: &Signal<u32>,
    #[cfg(feature = "pro")] stocks_text: &Signal<String>,
) {
    let container = container.clone();
    let results = results.clone();
    let units = units.clone();
    let parts_text = parts_text.clone();
    let opt_count = opt_count.clone();
    #[cfg(feature = "pro")]
    let stocks_text = stocks_text.clone();
    pick_text_file(
        ".json,application/json",
        Rc::new(move |text| {
            let Some(job) = project::read_project_json(&text) else {
                results.set(Outcome::Invalid(vec![Issue {
                    line: None,
                    message: "This file is not a Cut List Optimizer job (.cutlist.json)."
                        .to_string(),
                }]));
                return;
            };
            let job_units = Units::from_name(&job.units);
            storage_set("cutlist-units", job_units.name());
            units.set(job_units);
            write_field(&container, ".cutlist-parts", &job.parts);
            parts_text.set(job.parts);
            #[cfg(feature = "pro")]
            {
                write_field(&container, ".cutlist-stocks", &job.stocks);
                stocks_text.set(job.stocks);
            }
            #[cfg(not(feature = "pro"))]
            write_field(&container, ".cutlist-stock", &job.stocks);
            write_field(&container, ".cutlist-kerf", &job.kerf);
            run_optimize(&container, &results, &units, &opt_count);
        }),
    );
}

/// Downloads the placement table as CSV via a data-URI anchor click.
fn export_csv(results: &Signal<Outcome>, units: &Signal<Units>) {
    let outcome = results.get();
    if !matches!(outcome, Outcome::Solved { .. }) {
        return;
    }
    let units_now = units.get();

    let mut csv = String::from("# TPT Cut List Optimizer\n");
    csv.push_str(&grid_columns().join(","));
    csv.push('\n');
    for row in placement_rows(&outcome, units_now) {
        csv.push_str(
            &row.iter()
                .map(|cell| escape_csv(cell))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    download_data_uri("cutlist.csv", "text/csv", &csv);
}

/// Renders every bar as a self-contained SVG (proportional diagrams, labels
/// where they fit, native `<title>` tooltips) for quotes and emails.
fn export_svg(results: &Signal<Outcome>, units: &Signal<Units>) {
    let outcome = results.get();
    let Outcome::Solved {
        solution,
        parts,
        stocks,
        material_names,
        kerf,
    } = &outcome
    else {
        return;
    };
    let units_now = units.get();

    const WIDTH: f64 = 940.0;
    const MARGIN: f64 = 20.0;
    const TRACK_W: f64 = 900.0;
    const TRACK_H: f64 = 26.0;
    const BAR_STEP: f64 = 62.0;
    let height = 72.0 + solution.bars.len() as f64 * BAR_STEP + 28.0;

    let palette = [
        "#4f8ef7", "#f2a33c", "#54b26a", "#e4636f", "#8f6fd6", "#2fb6c9", "#c9a227", "#7d8896",
    ];
    let stock_total: u64 = solution
        .bars
        .iter()
        .map(|bar| u64::from(stocks[bar.stock_index].length))
        .sum();
    let headline = format!(
        "Cut List \u{b7} {} bar{}, {} stock, {:.1}% utilization, kerf {}",
        solution.bar_count(),
        if solution.bar_count() == 1 { "" } else { "s" },
        fmt_len_u64(stock_total, units_now),
        solution.utilization * 100.0,
        fmt_len(*kerf, units_now)
    );

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{height}\" \
         viewBox=\"0 0 {WIDTH} {height}\" font-family=\"system-ui,'Segoe UI',sans-serif\">",
    );
    svg.push_str(&format!(
        "<text x=\"{MARGIN}\" y=\"26\" font-size=\"15\" font-weight=\"700\" fill=\"#1f2430\">{}</text>",
        xml_escape(&headline)
    ));

    for (bar_index, bar) in solution.bars.iter().enumerate() {
        let stock = &stocks[bar.stock_index];
        let y = 52.0 + bar_index as f64 * BAR_STEP;
        svg.push_str(&format!(
            "<text x=\"{MARGIN}\" y=\"{}\" font-size=\"11\" fill=\"#667085\">{}</text>",
            y + 9.0,
            xml_escape(&bar_header_text(bar_index, bar, stocks, material_names, *kerf, units_now))
        ));
        let track_y = y + 14.0;
        svg.push_str(&format!(
            "<rect x=\"{MARGIN}\" y=\"{track_y}\" width=\"{TRACK_W}\" height=\"{TRACK_H}\" rx=\"5\" fill=\"#e4e8ef\"/>"
        ));
        let mut x = MARGIN;
        for cut in &bar.cuts {
            let w = TRACK_W * f64::from(cut.length) / f64::from(stock.length);
            let color = palette[cut.part % palette.len()];
            let label = display_label(&parts[cut.part].label, cut.part);
            svg.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{track_y:.1}\" width=\"{w:.1}\" height=\"{TRACK_H}\" fill=\"{color}\"><title>{}</title></rect>",
                xml_escape(&format!(
                    "{label} \u{b7} {} at {}",
                    fmt_len(cut.length, units_now),
                    fmt_len(cut.offset, units_now)
                ))
            ));
            if w >= 96.0 {
                let mut text = label.clone();
                if parts[cut.part].grain {
                    text.push_str(" [g]");
                }
                if w >= 170.0 {
                    text.push_str(&format!(" \u{b7} {}", fmt_len_plain(cut.length, units_now)));
                }
                svg.push_str(&format!(
                    "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"10\" fill=\"#ffffff\">{}</text>",
                    x + 5.0,
                    track_y + 17.0,
                    xml_escape(&text)
                ));
            }
            x += w;
        }
        let waste_w = MARGIN + TRACK_W - x;
        if waste_w > 0.5 {
            svg.push_str(&format!(
                "<rect x=\"{x:.1}\" y=\"{track_y:.1}\" width=\"{waste_w:.1}\" height=\"{TRACK_H}\" rx=\"5\" fill=\"#c9ced9\"><title>{}</title></rect>",
                xml_escape(&format!(
                    "Offcut \u{b7} {}",
                    fmt_len(bar.waste(stock.length, *kerf, 0), units_now)
                ))
            ));
        }
    }

    svg.push_str(&format!(
        "<text x=\"{MARGIN}\" y=\"{:.0}\" font-size=\"10\" fill=\"#98a2b3\">{}</text>",
        height - 10.0,
        xml_escape(ENGINE_NOTE)
    ));
    svg.push_str("</svg>");
    download_data_uri("cutlist.svg", "image/svg+xml", &svg);
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Triggers a client-side download of `content` via a data-URI anchor.
fn download_data_uri(filename: &str, mime: &str, content: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(anchor) = document.create_element("a") else {
        return;
    };
    let uri = format!(
        "data:{mime};charset=utf-8,{}",
        percent_encode(content.as_bytes())
    );
    let _ = anchor.set_attribute("href", &uri);
    let _ = anchor.set_attribute("download", filename);
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        if let Some(link) = anchor.dyn_ref::<web_sys::HtmlAnchorElement>() {
            link.click();
        }
        let _ = body.remove_child(&anchor);
    }
}

fn escape_csv(cell: &str) -> String {
    if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Reads a value from localStorage (None on any error).
fn storage_get(key: &str) -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item(key).ok().flatten()
}

/// Writes a value to localStorage (best-effort).
fn storage_set(key: &str, value: &str) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item(key, value);
        }
    }
}
