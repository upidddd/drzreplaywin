# DrzReplayWIN installer / updater
#
#   irm https://github.com/@REPO@/releases/latest/download/install.ps1 | iex
#
# Downloads the latest setup and installs it for the current user (no admin rights needed).
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'      # the progress bar makes downloads 10x slower in Windows PowerShell
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$url = 'https://github.com/@REPO@/releases/latest/download/DrzReplayWIN-Setup.exe'
$setup = Join-Path $env:TEMP 'DrzReplayWIN-Setup.exe'
$updating = Test-Path (Join-Path $env:LOCALAPPDATA 'Programs\DrzReplayWIN\DrzReplayWIN.exe')

Write-Host 'Downloading DrzReplayWIN...'
Invoke-WebRequest -Uri $url -OutFile $setup -UseBasicParsing
Write-Host 'Installing...'
$p = Start-Process -FilePath $setup -ArgumentList '/SILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CLOSEAPPLICATIONS' -Wait -PassThru
Remove-Item $setup -ErrorAction SilentlyContinue
if ($p.ExitCode -ne 0) {
    Write-Host "Setup failed (code $($p.ExitCode))." -ForegroundColor Red
    return
}
if ($updating) {
    Write-Host 'Updated! Settings and clips are kept.' -ForegroundColor Green
} else {
    Write-Host 'Done! DrzReplayWIN is recording - press Alt+Z to save a replay.' -ForegroundColor Green
}
