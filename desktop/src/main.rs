//! TPT Cut List Optimizer Pro — desktop shell (paid edition).
//!
//! A native host binary that opens the OS webview (WebView2 on Windows) and
//! serves the `trunk build --release` output of the appfront DOM bundle from
//! `dist/`. The same DOM bundle ships to the hub for the free WASM edition;
//! the Pro difference at runtime is that the bundle is built with
//! `--features pro`, which compiles the solver's improvement pass in.
//!
//! Resolve order for the bundle directory:
//! 1. `TPT_CUTLIST_DIST` environment variable,
//! 2. `./dist` next to the current working directory,
//! 3. `dist` next to the executable.

use std::path::PathBuf;

use tpt_appfront_webview::{AppBuilder, WindowConfig};

const APP_ID: &str = "nz.co.tptsolutions.cutlist-optimizer-pro";

fn main() {
    let dist_dir = resolve_dist_dir();
    if !dist_dir.join("index.html").exists() {
        eprintln!(
            "tpt-cutlist-optimizer-pro: no index.html under {} — run \
             `trunk build --release` first (or set TPT_CUTLIST_DIST)",
            dist_dir.display()
        );
        std::process::exit(1);
    }

    let result = AppBuilder::new(APP_ID)
        .with_window(WindowConfig {
            id: "main".to_string(),
            title: "TPT Cut List Optimizer Pro".to_string(),
            width: 1100,
            height: 800,
            dist_dir,
        })
        .with_single_instance(true)
        .run(|_action, _params| Ok(()));

    if let Err(error) = result {
        eprintln!("tpt-cutlist-optimizer-pro: {error:#}");
        std::process::exit(1);
    }
}

fn resolve_dist_dir() -> PathBuf {
    if let Ok(from_env) = std::env::var("TPT_CUTLIST_DIST") {
        return PathBuf::from(from_env);
    }
    let cwd_dist = PathBuf::from("dist");
    if cwd_dist.join("index.html").exists() {
        return cwd_dist;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let exe_dist = exe_dir.join("dist");
            if exe_dist.join("index.html").exists() {
                return exe_dist;
            }
        }
    }
    cwd_dist
}
