# PrinterTUI installer for Windows 10/11.
#   irm https://raw.githubusercontent.com/s0artak/PrinterTUI/main/install.ps1 | iex
# Installs printertui.exe for this user (no administrator needed), puts it on the PATH and in the
# Start menu. Run it again to update or uninstall.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue' # the progress bar makes Invoke-WebRequest very slow

$repo = 's0artak/PrinterTUI'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\PrinterTUI'
$exe = Join-Path $dir 'printertui.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'PrinterTUI.lnk'

function Say($text) { Write-Host "  > $text" -ForegroundColor Cyan }

function Set-UserPath([string[]]$parts) {
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
}

function Install-PrinterTUI {
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
    $url = "https://github.com/$repo/releases/latest/download/printertui-windows-$arch.exe"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "printertui-$PID.exe"
    Say "Downloading PrinterTUI (windows $arch)"
    try {
        Invoke-WebRequest $url -OutFile $tmp -UseBasicParsing
        $want = ((Invoke-WebRequest "$url.sha256" -UseBasicParsing).Content -split '\s+')[0]
    } catch {
        Remove-Item $tmp -ErrorAction SilentlyContinue
        Write-Host "`n  Paper jam! The download didn't come through. Check your connection and try again." -ForegroundColor Red
        return
    }
    if ((Get-FileHash $tmp -Algorithm SHA256).Hash -ne $want.ToUpper()) {
        Remove-Item $tmp
        Write-Host "`n  Smudged page! The download doesn't match its checksum, nothing was installed." -ForegroundColor Red
        return
    }
    New-Item -ItemType Directory -Force $dir | Out-Null
    Move-Item -Force $tmp $exe

    # on the PATH for new terminals, and for this one right away
    $path = [Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ }
    if ($path -notcontains $dir) { Set-UserPath ($path + $dir) }
    if (($env:Path -split ';') -notcontains $dir) { $env:Path += ";$dir" }

    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    $link.TargetPath = $exe
    $link.WorkingDirectory = [Environment]::GetFolderPath('MyDocuments')
    $link.Save()

    Write-Host "`n  Page printed, ink dry, ready to go. Run: printertui (or PrinterTUI in the Start menu)`n" -ForegroundColor Green
    Say 'Optional: LibreOffice (libreoffice.org) to print Word, Excel and other non-PDF files'
}

function Uninstall-PrinterTUI {
    Say 'Removing PrinterTUI, its settings and its copy of pdfium'
    Get-Process printertui -ErrorAction SilentlyContinue | Stop-Process -Force
    Remove-Item -Recurse -Force $dir, (Join-Path $env:APPDATA 'printertui'), (Join-Path $env:LOCALAPPDATA 'printertui') -ErrorAction SilentlyContinue
    Remove-Item $shortcut -ErrorAction SilentlyContinue
    Set-UserPath ([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ -and $_ -ne $dir })
    Write-Host "`n  Paper's back in the tray. Bye for now! Your printers were left as they are.`n" -ForegroundColor Green
}

Write-Host "`n  PrinterTUI`n" -ForegroundColor White
if (Test-Path $exe) {
    Write-Host "  Oh, you already have me. Fresh copy, or shall I pack up?`n" -ForegroundColor Cyan
    Write-Host '    1  Update'
    Write-Host '    2  Uninstall'
    Write-Host "    q  Quit`n"
    switch (Read-Host '  Pick') {
        '1' { Install-PrinterTUI }
        '2' { Uninstall-PrinterTUI }
        default { Write-Host "`n  No worries, nothing was touched.`n" }
    }
} else {
    Write-Host "  Hi! Looks like we haven't met. Warm up the printer?`n" -ForegroundColor Cyan
    Install-PrinterTUI
}
