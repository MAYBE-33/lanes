# Build everything that ships, into dist\.
#
#     powershell -ExecutionPolicy Bypass -File tools\build-release.ps1
#
# The result is a folder you could zip and hand to someone. The installer in
# installer\ packages this same folder, so what is tested portable and what is
# installed are byte-identical.
#
# Needs: the Rust toolchain pinned in rust-toolchain.toml (rustup installs it),
# the MSVC build tools, and the .NET 10 SDK. See docs\building.md.
#
# WHY THE STATIC CRT MATTERS HERE
#
# The Rust core links the C runtime statically (see .cargo\config.toml). Built
# any other way it imports vcruntime140.dll, which comes from the Visual C++
# redistributable: present on most developer machines, and absent on a clean
# Windows install. That is the worst kind of dependency,
# because it works everywhere it is tested. This script verifies the result
# rather than trusting the setting.

param(
    [string]$Out = "dist",
    [switch]$SkipWindow
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$dist = Join-Path $root $Out
if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force -Path $dist | Out-Null

Write-Host "==> core (Rust)" -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
Copy-Item "$root\target\release\Lanes.exe" $dist

if (-not $SkipWindow) {
    Write-Host "==> window (C# / WPF)" -ForegroundColor Cyan
    # Self-contained: the .NET runtime travels inside the executable, so there
    # is nothing for the person installing this to download first.
    dotnet publish "$root\ui-wpf\Lanes.Window.csproj" -c Release -o $dist --nologo
    if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed" }

    # publish drops build leftovers beside the binary; only the exe ships.
    Get-ChildItem $dist -File |
        Where-Object { $_.Name -notin 'Lanes.exe', 'Lanes.Window.exe' } |
        Remove-Item -Force
}

Write-Host "==> checking what the binaries depend on" -ForegroundColor Cyan
$bad = $false
foreach ($exe in Get-ChildItem $dist -Filter *.exe) {
    $text = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($exe.FullName))

    # vcruntime140_cor3.dll is .NET's own copy, carried inside the self-contained
    # bundle and extracted at run time. It is not an external dependency, which
    # is why it is excluded by name rather than by pattern.
    $needs = [regex]::Matches($text, '(?i)\b(vcruntime\w*|msvcp\w*|msvcr\w*)\.dll\b') |
             ForEach-Object { $_.Value.ToLower() } |
             Where-Object { $_ -ne 'vcruntime140_cor3.dll' } |
             Sort-Object -Unique

    $mb = [math]::Round($exe.Length / 1MB, 2)
    if ($needs) {
        Write-Host ("    {0,-26} {1,6} MB   NEEDS {2}" -f $exe.Name, $mb, ($needs -join ', ')) -ForegroundColor Red
        $bad = $true
    } else {
        Write-Host ("    {0,-26} {1,6} MB   no redistributable needed" -f $exe.Name, $mb) -ForegroundColor Green
    }
}
if ($bad) {
    throw "a shipped binary needs the Visual C++ redistributable; check .cargo\config.toml"
}

Write-Host ""
Write-Host "dist: $dist" -ForegroundColor Cyan
Get-ChildItem $dist | Select-Object Name, @{n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } } | Format-Table -AutoSize
