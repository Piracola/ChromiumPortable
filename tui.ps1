# ChromiumPortable 构建菜单（双击 构建.bat 进入）
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Show-Menu {
    Clear-Host
    Write-Host ""
    Write-Host "  ChromiumPortable 构建台" -ForegroundColor Cyan
    Write-Host "  ----------------------"
    Write-Host ""
    Write-Host "  [1] 快速构建      试界面用，最快"
    Write-Host "  [2] 正式发行包    dist\ 下出可分发文件夹"
    Write-Host "  [3] 跑全部测试    引擎 + 界面共 159 个"
    Write-Host "  [4] 清理缓存      target\ 瘦身"
    Write-Host "  [5] 打开图形界面"
    Write-Host "  [0] 退出"
    Write-Host ""
}

while ($true) {
    Show-Menu
    $choice = Read-Host "  选一个，回车确认"
    switch ($choice) {
        "1" { & "$PSScriptRoot\build.ps1" }
        "2" { & "$PSScriptRoot\build.ps1" -Release }
        "3" { & "$PSScriptRoot\build.ps1" -Test }
        "4" { & "$PSScriptRoot\build.ps1" -Clean }
        "5" {
            $exe = Join-Path $PSScriptRoot "target\debug\builder-app.exe"
            if (-not (Test-Path $exe)) {
                Write-Host "  还没构建过，先来一次快速构建……" -ForegroundColor Yellow
                & "$PSScriptRoot\build.ps1"
            }
            if (Test-Path $exe) {
                Start-Process $exe
                Write-Host "  界面已打开。"
            } else {
                Write-Host "  构建失败，没打开。" -ForegroundColor Red
            }
        }
        "0" { return }
    }
    if ($choice -ne "0") {
        Write-Host ""
        Read-Host "  完了，按回车回菜单" | Out-Null
    }
}
