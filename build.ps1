# ChromiumPortable 构建脚本
# 平时用：       .\构建.bat        （debug，快，试界面用）
# 出发行包：     .\构建.bat -Release
# 顺手跑测试：   .\构建.bat -Test
# 清缓存：       .\构建.bat -Clean
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

# ---- 发行包：与 CI（release-engine.yml）同一套清单 ----
$stage = "dist\ChromiumPortableBuilder-windows-x64"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item "$bin\builder-app.exe" $stage/
Copy-Item "$bin\portable-builder.exe" $stage/
Copy-Item catalog $stage/catalog -Recurse
Copy-Item setdll $stage/setdll -Recurse
if (Test-Path "7zr.exe") {
    Copy-Item 7zr.exe $stage/
} else {
    try {
        Invoke-WebRequest -Uri 'https://www.7-zip.org/a/7zr.exe' -OutFile "$stage\7zr.exe" -UseBasicParsing
    } catch {
        Write-Host "[i] 7zr.exe 没拿到（不影响构建，引擎首跑会自己下载）" -ForegroundColor Yellow
    }
}
$size = "{0:N1} MB" -f ((Get-ChildItem $stage -Recurse | Measure-Object Length -Sum).Sum / 1MB)
Write-Host ""
Write-Host "[OK] 发行包就绪：$stage  ($size)" -ForegroundColor Green
Write-Host "     整个文件夹打包 zip 就能发给别人。"
