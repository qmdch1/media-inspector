$outDir = "$PSScriptRoot\dist"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

cargo build --release --manifest-path "$PSScriptRoot\Cargo.toml"

if ($LASTEXITCODE -eq 0) {
    Copy-Item "$PSScriptRoot\target\release\MediaInspector.exe" "$outDir\MediaInspector.exe" -Force
    Copy-Item "$PSScriptRoot\icon.ico" "$outDir\icon.ico" -Force -ErrorAction SilentlyContinue
    Write-Host "Build successful: $outDir\MediaInspector.exe" -ForegroundColor Green
} else {
    Write-Host "Build failed" -ForegroundColor Red
}
