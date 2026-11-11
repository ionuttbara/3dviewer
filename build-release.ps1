$ErrorActionPreference = "Stop"
$env:RUST_BACKTRACE = "1"
if (-not (Test-Path "assets\app.ico")) { throw "Missing application icon: assets\app.ico" }
if (-not (Test-Path "assets\model-file.ico")) { throw "Missing model file icon: assets\model-file.ico" }
$shader = Get-Content "src\shader.wgsl" -Raw
if ($shader -match '\.[xyzwrgba]{2,4}\s*=(?!=)') {
    throw "WGSL preflight failed: assignments to vector swizzles (for example base.rgb = ...) are not supported. Assign a complete vector instead."
}
Write-Host "WGSL preflight: OK" -ForegroundColor DarkGreen
cargo build --release
Write-Host ""
Write-Host "Built: target\release\gallery-3d-viewer.exe" -ForegroundColor Green
