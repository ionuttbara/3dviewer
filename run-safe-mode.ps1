$ErrorActionPreference = "Stop"
$env:RUST_BACKTRACE = "full"
Write-Host "3D Viewer 0.2.2 - safe diagnostic mode (CAD/OpenCASCADE not linked)" -ForegroundColor Yellow
cargo run --release --no-default-features
