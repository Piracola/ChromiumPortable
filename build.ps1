param(
    [switch]$Release,
    [switch]$Test,
    [switch]$Clean
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "[!] 没找到 cargo。先装 Rust：https://rustup.rs" -ForegroundColor Red
    exit 1
}

if ($Clean) {
    cargo clean
    Write-Host "[OK] target\ 缓存已清空。"
    return
}

if ($Test) {
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { Write-Host "[!] 测试未通过，停止。" -ForegroundColor Red; exit 1 }
}

$flags = @()
if ($Release) { $flags += "--release" }
cargo build @flags -p portable-builder -p builder-app
if ($LASTEXITCODE -ne 0) { exit 1 }

$bin = if ($Release) { "target\release" } else { "target\debug" }
Write-Host ""
Write-Host "[OK] 引擎 $($bin)\portable-builder.exe"
Write-Host "[OK] 界面 $($bin)\builder-app.exe"

if (-not $Release) {
    Write-Host "（加 -Release 出可分发文件夹）"
    return
}

$stage = "dist\ChromiumPortableBuilder-windows-x64"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
& (Join-Path $PSScriptRoot "scripts\stage-dist.ps1") -Stage $stage -Bin $bin -Gui
$size = "{0:N1} MB" -f ((Get-ChildItem $stage -Recurse | Measure-Object Length -Sum).Sum / 1MB)
Write-Host ""
Write-Host "[OK] 发行包就绪：$stage  ($size)" -ForegroundColor Green
Write-Host "     整个文件夹打包 zip 就能发给别人。"
