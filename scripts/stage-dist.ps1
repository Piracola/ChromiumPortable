# 发行包暂存：build.ps1 与 release-engine.yml 共用这一份清单
param(
    [Parameter(Mandatory = $true)] [string] $Stage,
    [Parameter(Mandatory = $true)] [string] $Bin,
    [switch] $Gui
)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent

New-Item -ItemType Directory -Force -Path $Stage | Out-Null
if ($Gui) { Copy-Item "$Bin\builder-app.exe" $Stage/ }
Copy-Item "$Bin\portable-builder.exe" $Stage/
Copy-Item (Join-Path $repo catalog) $Stage/catalog -Recurse
Copy-Item (Join-Path $repo setdll) $Stage/setdll -Recurse
if (Test-Path (Join-Path $repo 7zr.exe)) {
    Copy-Item (Join-Path $repo 7zr.exe) $Stage/
} else {
    try {
        Invoke-WebRequest -Uri 'https://www.7-zip.org/a/7zr.exe' -OutFile "$Stage\7zr.exe" -UseBasicParsing
    } catch {
        Write-Host "[i] 7zr.exe 没拿到（不影响构建，引擎首跑会自己下载）" -ForegroundColor Yellow
    }
}
