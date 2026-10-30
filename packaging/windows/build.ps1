# Build Windows release binary + Inno Setup installer.
# Prerequisites: Rust 1.87+, Inno Setup 6 (ISCC.exe on PATH or default install path).
# Usage (from repo root):  powershell -ExecutionPolicy Bypass -File packaging/windows/build.ps1

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $Root

$CargoToml = Get-Content -Raw "Cargo.toml"
if ($CargoToml -match 'version\s*=\s*"([^"]+)"') { $Version = $Matches[1] } else { $Version = "0.1.0" }

Write-Host "==> cargo build --release"
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$Dist = Join-Path $Root "dist\windows"
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
# Cargo package name is `pfd` → target/release/pfd.exe; stage user-facing PFD.exe
Copy-Item -Force "target\release\pfd.exe" (Join-Path $Dist "PFD.exe")

$Iscc = $null
foreach ($cand in @(
    "ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
)) {
    if (Get-Command $cand -ErrorAction SilentlyContinue) { $Iscc = (Get-Command $cand).Source; break }
    if (Test-Path $cand) { $Iscc = $cand; break }
}
if (-not $Iscc) {
    Write-Warning "ISCC.exe not found. Staged binary at $Dist\PFD.exe"
    Write-Warning "Install Inno Setup 6 and re-run, or: ISCC packaging\windows\pfd.iss"
    exit 0
}

$Iss = Join-Path $Root "packaging\windows\pfd.iss"
Write-Host "==> $Iscc $Iss (version $Version)"
& $Iscc "/DMyAppVersion=$Version" "/DDistDir=$Dist" $Iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

Write-Host "Done. Installer under $Dist"
Get-ChildItem $Dist | Format-Table Name, Length
