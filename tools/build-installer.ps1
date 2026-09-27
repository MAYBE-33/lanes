# Build the installer.
#
#     powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
#
# Builds dist\ first, so the installer always packages a fresh build rather
# than whatever happened to be sitting there. Output lands in dist\installer\.
#
# Needs Inno Setup 6:
#     winget install --id JRSoftware.InnoSetup

param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $SkipBuild) {
    & "$PSScriptRoot\build-release.ps1"
}

$iscc = @(
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $iscc) {
    throw "Inno Setup 6 not found. Install it with: winget install --id JRSoftware.InnoSetup"
}

Write-Host "==> installer" -ForegroundColor Cyan
& $iscc /Q "$root\installer\Lanes.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with $LASTEXITCODE" }

Get-ChildItem "$root\dist\installer" -Filter *.exe |
    Select-Object Name, @{n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } } |
    Format-Table -AutoSize
