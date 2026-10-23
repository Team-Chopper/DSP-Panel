# Build the release package of the Rust panel (dsp-panel-rs) on Windows.
#
#   .\rust\tool\build_dist.ps1                 x64 package
#   .\rust\tool\build_dist.ps1 -Arch arm64     arm64 package
#
# Result: rust\dist\dsp-panel-rs-<version>-linux-<arch>.tar.gz with the
# server (static musl binary), the web client and the install scripts.
# Install on the VM:
#
#   scp rust\dist\dsp-panel-rs-<version>-linux-x64.tar.gz user@vm:/tmp/
#   ssh user@vm
#   tar xzf /tmp/dsp-panel-rs-<version>-linux-x64.tar.gz && cd dsp-panel-rs-<version>-linux-x64
#   sudo bash install.sh --import-from /var/lib/dsp-panel
#
# The Linux binary is cross-compiled with cargo-zigbuild (needs zig and
# cargo-zigbuild on PATH). The web client (crates\dsp-web) is built with trunk;
# -Web <folder> packages another build instead (e.g. -Web build\web for the
# Flutter client).

param(
  [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
  [string]$Web,
  # Also publish the package as rust\release\dsp-panel-rs-linux-<arch>.tar.gz
  # with rust\release\latest-<arch>.json (version, sha256) for automatic
  # installation from the repository.
  [switch]$Release
)

$ErrorActionPreference = 'Stop'
$rust = Split-Path -Parent $PSScriptRoot
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target\dsp-panel" }

function Invoke-Checked([scriptblock]$block, [string]$what) {
  # Native tools report progress on stderr; only the exit code counts.
  $previous = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { & $block 2>&1 | ForEach-Object { "$_" } | Out-Host } finally { $ErrorActionPreference = $previous }
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)" }
}

$triple = if ($Arch -eq 'x64') { 'x86_64-unknown-linux-musl' } else { 'aarch64-unknown-linux-musl' }
$version = ((Select-String -Path "$rust\Cargo.toml" -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value)
$name = "dsp-panel-rs-$version-linux-$Arch"
$out = Join-Path $rust "dist\$name"
$archive = Join-Path $rust "dist\$name.tar.gz"

Write-Host "==> Building $name"
if (-not $Web) {
  Write-Host "==> Web client (trunk)"
  Push-Location "$rust\crates\dsp-web"
  try { Invoke-Checked { trunk build --release } 'trunk build' } finally { Pop-Location }
  $Web = "$rust\crates\dsp-web\dist"
}
if (-not (Test-Path "$Web\index.html")) { throw "Web client not found in $Web" }

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
if (Test-Path $archive) { Remove-Item -Force $archive }
New-Item -ItemType Directory -Force $out | Out-Null

Write-Host "==> Server ($triple)"
Push-Location $rust
try {
  Invoke-Checked { rustup target add $triple } 'rustup target add'
  Invoke-Checked { cargo zigbuild --release -p dsp-server --target $triple } 'cargo zigbuild'
} finally { Pop-Location }
Copy-Item "$env:CARGO_TARGET_DIR\$triple\release\dsp-server" "$out\dsp-server"

Write-Host "==> Web client ($Web)"
Copy-Item -Recurse $Web "$out\web"
Remove-Item -Force -ErrorAction SilentlyContinue "$out\web\.trunk-lock"

# Scripts with LF line endings (a checkout with autocrlf may have CRLF)
foreach ($f in 'install.sh', 'uninstall.sh', 'dsp-panel-rs.service', 'config.example.yaml') {
  $text = [System.IO.File]::ReadAllText("$rust\deploy\$f") -replace "`r`n", "`n"
  [System.IO.File]::WriteAllText("$out\$f", $text, (New-Object System.Text.UTF8Encoding $false))
}

Invoke-Checked { tar -C (Join-Path $rust 'dist') -czf $archive $name } 'tar'
Write-Host "==> rust\dist\$name.tar.gz"

if ($Release) {
  $releaseDir = Join-Path $rust 'release'
  New-Item -ItemType Directory -Force $releaseDir | Out-Null
  $file = "dsp-panel-rs-linux-$Arch.tar.gz"
  Copy-Item $archive (Join-Path $releaseDir $file) -Force
  $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $releaseDir $file)).Hash.ToLower()
  $manifest = [ordered]@{
    name    = 'dsp-panel-rs'
    version = $version
    arch    = $Arch
    file    = $file
    folder  = $name
    sha256  = $hash
    size    = (Get-Item (Join-Path $releaseDir $file)).Length
    built   = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    install = 'sudo bash install.sh [--import-from /var/lib/dsp-panel]'
  }
  $json = ($manifest | ConvertTo-Json) -replace "`r`n", "`n"
  [System.IO.File]::WriteAllText((Join-Path $releaseDir "latest-$Arch.json"), "$json`n", (New-Object System.Text.UTF8Encoding $false))
  Write-Host "==> rust\release\$file (sha256 $hash)"
}
Write-Host "    On the VM (Windows tar does not keep the executable bits): sudo bash install.sh [--import-from /var/lib/dsp-panel]"
