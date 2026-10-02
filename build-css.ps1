# Builds static/css/tailwind.css with the standalone Tailwind CLI (no Node needed).
#   .\build-css.ps1          one-off build
#   .\build-css.ps1 -Watch   rebuild whenever a template changes
param([switch]$Watch)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$version = "v3.4.17"   # the project uses Tailwind v3 syntax; do not use the v4 CLI
$exe = Join-Path $PSScriptRoot "tailwindcss.exe"

if (-not (Test-Path $exe)) {
    Write-Host "Downloading Tailwind CLI $version ..."
    Invoke-WebRequest "https://github.com/tailwindlabs/tailwindcss/releases/download/$version/tailwindcss-windows-x64.exe" -OutFile $exe
}

$args = @("-c", "tailwind.config.js", "-i", "assets/input.css", "-o", "static/css/tailwind.css")
if ($Watch) { $args += "--watch" } else { $args += "--minify" }
& $exe @args
