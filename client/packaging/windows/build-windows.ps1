# Build the portable Windows package (.zip with paddock.exe). Run in PowerShell
# from any folder, on Windows with Rust installed (https://rustup.rs).
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..\..")
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
cargo build --release
New-Item -ItemType Directory -Force dist | Out-Null
$stage = Join-Path $env:TEMP "paddock-$version"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item target\release\paddock-client.exe (Join-Path $stage "Paddock.exe")
Compress-Archive -Force -Path "$stage\*" -DestinationPath "dist\paddock-$version-windows-x64.zip"
Write-Host "OK  dist\paddock-$version-windows-x64.zip"
