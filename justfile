# TPT Cut List Optimizer - task runner.
# The pipeline logic lives in build.ps1 (Windows) / build.sh (unix); these
# recipes are thin wrappers so `just` works everywhere.
#
# Outputs (never overwrite each other):
#   dist\hub\      free wasm bundle for the website
#   dist\desktop\  trunk bundle the Pro exe serves
#   release\       staged Pro exe package + zip (-Pro / just pro)

set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Build the free WASM edition into dist\hub (+ gzip size report).
build:
    pwsh -NoLogo -NoProfile -File build.ps1

# Build the Pro edition: improved multi-material solver, trunk bundle in
# dist\desktop, exe, and a zip-ready package in release\.
pro:
    pwsh -NoLogo -NoProfile -File build.ps1 -Pro

# Copy the hub bundle into the web repo (set TPT_WEB_REPO first).
deploy:
    @if (-not $env:TPT_WEB_REPO) { Write-Error "set `$env:TPT_WEB_REPO to the web repo checkout first"; exit 1 }
    pwsh -NoLogo -NoProfile -File build.ps1 -SkipBuild -WebRepo:$env:TPT_WEB_REPO

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
    -rd /s /q release 2>nul || rm -rf release

