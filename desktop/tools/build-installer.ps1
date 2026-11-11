# Builds the Telegram page, the release executable and the installer.
# The installer lands outside the repository: it contains client-access.json.
param(
    [string]$TargetDir = "C:\Dev\tmp\tbs-target",
    [string]$OutputDir = "C:\Dev\tmp\tbs-installer"
)
$ErrorActionPreference = "Stop"
$desktop = Split-Path -Parent $PSScriptRoot
$frontend = Join-Path (Split-Path -Parent $desktop) "frontend"

$manifest = Get-Content (Join-Path $desktop "Cargo.toml") -Raw
if ($manifest -notmatch '(?m)^version = "([^"]+)"') { throw "no workspace version in Cargo.toml" }
$version = $Matches[1]

Push-Location $frontend
try {
    npm run build:telegram
    if ($LASTEXITCODE -ne 0) { throw "the Telegram page did not build" }
} finally { Pop-Location }

$env:CARGO_TARGET_DIR = $TargetDir
Push-Location $desktop
try {
    cargo build -p tbs-app --release
    if ($LASTEXITCODE -ne 0) { throw "the client did not build" }
} finally { Pop-Location }

New-Item -ItemType Directory -Force $OutputDir | Out-Null
iscc "/DAppVersion=$version" "/DBuildDir=$(Join-Path $TargetDir 'release')" "/DOutputDirectory=$OutputDir" (Join-Path $desktop "installer\tbsparcer.iss")
if ($LASTEXITCODE -ne 0) { throw "the installer did not build" }

$setup = Join-Path $OutputDir "TBSparcer-Native-Setup-$version.exe"
"{0}  {1:N1} MB  sha256 {2}" -f $setup, ((Get-Item $setup).Length / 1MB), (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
