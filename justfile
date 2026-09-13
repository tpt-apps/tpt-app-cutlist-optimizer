# TPT Cut List Optimizer - task runner.
# The pipeline logic lives in build.ps1 (Windows) / build.sh (unix); these
# recipes are thin wrappers so `just` works everywhere.

set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Build the free WASM edition and report the gzip payload size.
build:
    pwsh -NoLogo -NoProfile -File build.ps1

# Build the Pro edition: improved solver + trunk bundle + desktop exe.
pro:
    pwsh -NoLogo -NoProfile -File build.ps1 -Pro

# Re-run only wasm-bindgen + copy (after an existing release build).
bindgen:
    pwsh -NoLogo -NoProfile -File build.ps1 -SkipBuild

# Host test suites: solver (baseline), solver (all features), parser.
test:
    cargo test -p tpt-cutlist-solver
    cargo test -p tpt-cutlist-solver --all-features
    cargo test -p tpt-app-cutlist-optimizer

# Type-check the wasm lib (free) and the desktop shell.
check:
    cargo check --target wasm32-unknown-unknown -p tpt-app-cutlist-optimizer
    cargo check --target wasm32-unknown-unknown --features pro -p tpt-app-cutlist-optimizer
    cargo check -p tpt-cutlist-desktop

# Live-reload standalone dev server (index.html mounts via #tpt-appfront-root).
dev:
    trunk serve

# Remove build outputs.
clean:
    cargo clean
    -rd /s /q dist 2>nul || rm -rf dist
