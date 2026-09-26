# Plays the Windows installer's menus and animations without installing, removing or downloading
# anything (works in PowerShell 7 on Linux and macOS too).
#   pwsh test/installer-preview.ps1            as on a machine that has PrinterTUI: Update / Uninstall
#   pwsh test/installer-preview.ps1 fresh      as on a first install: Install
#   pwsh test/installer-preview.ps1 jam        first install where the download fails: paper jam
#   pwsh test/installer-preview.ps1 smudge     first install where the checksum does not match
param([string]$mode = 'installed')
$env:PRINTERTUI_PREVIEW = $mode
Invoke-Expression (Get-Content -Raw -Encoding UTF8 (Join-Path (Split-Path $PSScriptRoot) 'install.ps1'))
