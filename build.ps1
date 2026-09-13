# TPT Cut List Optimizer - build pipeline (Windows).
#
# Free edition (default):  ./build.ps1
#   cargo build --release --target wasm32-unknown-unknown
#   wasm-bindgen --target web  ->  glue + wasm
#   copy into the web repo public/apps/cutlist-optimizer/ when TPT_WEB_REPO is
#   set (glue named cutlist-optimizer.js, wasm keeps its _bg.wasm name - the
#   glue resolves it via import.meta.url), otherwise into dist\web.
# Pro desktop edition:     ./build.ps1 -Pro
#   same, with --features pro (improvement search compiled in), plus
#   trunk build --release and the native webview exe.
#
# Env:
#   TPT_WEB_REPO  path to the tptsolutions.co.nz web repo; the app is copied
#                 to <repo>\public\apps\cutlist-optimizer\ for the hub runner.

[CmdletBinding()]
param(
    # Build the Pro bundle (--features pro) instead of the free one.
    [switch]$Pro,
    # Only run the wasm-bindgen step against an existing release build.
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

$Crate = "tpt-app-cutlist-optimizer"
$Target = "wasm32-unknown-unknown"
$OutName = "cutlist-optimizer"

# Where the browser-loadable bundle goes.
$WebRepo = $env:TPT_WEB_REPO
$Dest = if ($WebRepo) {
    Join-Path $WebRepo "public\apps\cutlist-optimizer"
} else {
    Join-Path $PSScriptRoot "dist\web"
}

Write-Host "== TPT Cut List Optimizer build ==" -ForegroundColor Cyan
Write-Host " edition: $(if ($Pro) { 'pro (improvement search enabled)' } else { 'free (baseline BFD only)' })"
Write-Host " dest:    $Dest"

if (-not $SkipBuild) {
    $features = if ($Pro) { "--features", "pro" } else { @() }
    Write-Host "-- cargo build --release --target $Target" -ForegroundColor Cyan
    if ($features.Count -gt 0) {
        cargo build --release --target $Target -p $Crate @features
    } else {
        cargo build --release --target $Target -p $Crate
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$Wasm = Join-Path $PSScriptRoot "target\$Target\release\tpt_app_cutlist_optimizer.wasm"
if (-not (Test-Path $Wasm)) { throw "missing $Wasm - run the cargo build step first" }

Write-Host "-- wasm-bindgen --target web --out-name $OutName" -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
wasm-bindgen --target web --out-name $OutName --out-dir $Dest $Wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

$Glue = Join-Path $Dest "$OutName.js"
$BgWasm = Join-Path $Dest "$($OutName)_bg.wasm"
foreach ($artifact in @($Glue, $BgWasm)) {
    if (-not (Test-Path $artifact)) { throw "wasm-bindgen did not produce $artifact" }
}

# Payload size report (todo Phase 4: target < ~400 KB gzipped).
$raw = (Get-Item $BgWasm).Length
Add-Type -AssemblyName System.IO.Compression
$rawBytes = [System.IO.File]::ReadAllBytes($BgWasm)
$memoryStream = [System.IO.MemoryStream]::new()
$gzipStream = [System.IO.Compression.GZipStream]::new($memoryStream, [System.IO.Compression.CompressionLevel]::Optimal)
$gzipStream.Write($rawBytes, 0, $rawBytes.Length)
$gzipStream.Dispose()
$gzipped = $memoryStream.ToArray().Length
$memoryStream.Dispose()
$gzKb = [math]::Round($gzipped / 1KB, 1)
Write-Host "-- $(Split-Path $BgWasm -Leaf): $raw bytes raw, $gzipped bytes gzipped ($gzKb KB gz)" -ForegroundColor Cyan
if ($gzipped -gt 400KB) {
    Write-Warning "payload exceeds the ~400 KB gzip target for first-load comfort"
}

if ($Pro) {
    Write-Host "-- trunk build --release --features pro (standalone bundle for the desktop shell)" -ForegroundColor Cyan
    trunk build --release --features pro
    if ($LASTEXITCODE -ne 0) { throw "trunk build failed" }

    Write-Host "-- cargo build --release -p tpt-cutlist-desktop (webview exe)" -ForegroundColor Cyan
    cargo build --release -p tpt-cutlist-desktop
    if ($LASTEXITCODE -ne 0) { throw "desktop build failed" }
    Write-Host "   exe: target\release\tpt-cutlist-optimizer-pro.exe (serves dist\)" -ForegroundColor Cyan
}

Write-Host "-- done" -ForegroundColor Green
if (-not $WebRepo) {
    Write-Host "   (set TPT_WEB_REPO to copy the bundle into the hub web repo)"
}
