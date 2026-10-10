# Build the release package on Windows and install it on the Ubuntu VM.
#
#   .\tool\deploy.ps1 -Target user@vm-ip          first time (target is remembered)
#   .\tool\deploy.ps1                             later
#   .\tool\deploy.ps1 -SkipBuild                  re-send the last package
#   .\tool\deploy.ps1 -BuildOnly                  only build dist\dsp-panel-<version>-linux-x64.tar.gz
#
# Same package as tool/build_dist.sh (web client + cross-compiled Linux
# server + install scripts). It is copied with scp to the VM, unpacked in
# ~/dsp-panel-install and installed with `sudo ./install.sh`, which keeps
# configuration, DSP list and password and restarts the service. ssh asks
# for the VM password (and sudo for its own) unless a key is set up.
#
# Needs Flutter (C:\src\flutter, or on PATH), git (on PATH or from GitHub
# Desktop) and the OpenSSH client of Windows.

param(
  [string]$Target,
  [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
  [switch]$SkipBuild,
  [switch]$BuildOnly
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Remember the target in an untracked file
$targetFile = Join-Path $root '.deploy-target'
if ($Target) { Set-Content -Path $targetFile -Value $Target -Encoding ascii }
elseif (Test-Path $targetFile) { $Target = (Get-Content $targetFile -TotalCount 1).Trim() }
if (-not $Target -and -not $BuildOnly) { throw 'No target: run once with -Target user@vm-ip' }

# Tools
if (-not (Get-Command flutter -ErrorAction SilentlyContinue) -and (Test-Path 'C:\src\flutter\bin\flutter.bat')) {
  $env:PATH = "C:\src\flutter\bin;$env:PATH"
}
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
  $desktop = Get-ChildItem "$env:LOCALAPPDATA\GitHubDesktop" -Directory -Filter 'app-*' -ErrorAction SilentlyContinue |
    Sort-Object Name -Descending | Select-Object -First 1
  if ($desktop) { $env:PATH = "$($desktop.FullName)\resources\app\git\cmd;$env:PATH" }
}

$version = ((Select-String -Path pubspec.yaml -Pattern '^version:\s*(.+)$').Matches[0].Groups[1].Value).Trim()
$name = "dsp-panel-$version-linux-$Arch"
$out = Join-Path $root "dist\$name"
$archive = Join-Path $root "dist\$name.tar.gz"

function Invoke-Checked([scriptblock]$block, [string]$what) {
  & $block
  if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)" }
}

if (-not $SkipBuild) {
  Write-Host "==> Building $name"
  if (Test-Path $out) { Remove-Item -Recurse -Force $out }
  if (Test-Path $archive) { Remove-Item -Force $archive }
  New-Item -ItemType Directory -Force $out | Out-Null

  Write-Host '==> Web client'
  # pub get may report a symlink error for the Windows plugin folder when the
  # Windows developer mode is off; the web build does not need it.
  flutter pub get | Out-Host
  Invoke-Checked { flutter build web --release --no-web-resources-cdn --pwa-strategy=none --no-wasm-dry-run } 'flutter build web'
  Copy-Item -Recurse build\web "$out\web"

  Write-Host "==> Server (linux-$Arch)"
  Push-Location server
  try {
    Invoke-Checked { dart pub get } 'dart pub get'
    Invoke-Checked { dart compile exe bin/dsp_server.dart --target-os=linux --target-arch=$Arch -o "$out\dsp-server" } 'dart compile'
  } finally { Pop-Location }

  Copy-Item deploy\install.sh, deploy\uninstall.sh, deploy\dsp-panel.service, deploy\config.example.yaml $out
  Invoke-Checked { tar -C (Join-Path $root 'dist') -czf $archive $name } 'tar'
  Write-Host "==> dist\$name.tar.gz"
}
if ($BuildOnly) { return }
if (-not (Test-Path $archive)) { throw "Package $archive not found (build first)" }

Write-Host "==> Copying to $Target"
Invoke-Checked { scp $archive "${Target}:/tmp/$name.tar.gz" } 'scp'

Write-Host "==> Installing on $Target"
# Windows tar does not keep the executable bits: set them on the VM.
$remote = "set -e; rm -rf ~/dsp-panel-install; mkdir -p ~/dsp-panel-install; " +
  "tar -xzf /tmp/$name.tar.gz -C ~/dsp-panel-install; rm /tmp/$name.tar.gz; " +
  "cd ~/dsp-panel-install/$name; chmod +x install.sh uninstall.sh dsp-server; " +
  "sudo ./install.sh; systemctl --no-pager status dsp-panel | head -n 5"
Invoke-Checked { ssh -t $Target $remote } 'remote install'
Write-Host "==> Done: dsp-panel $version is running on $($Target.Split('@')[-1])"
