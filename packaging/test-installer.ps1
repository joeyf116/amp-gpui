# Runs an installer through a real install, upgrade, launch, and uninstall on a
# clean machine. CI runs this before an installer is published.
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$OldInstaller,
    [Parameter(Mandatory)][string]$Version
)

$ErrorActionPreference = "Stop"

$appDir = "$env:LOCALAPPDATA\Programs\Amp GPUI"
$exe = "$appDir\amp-gpui.exe"
$startMenu = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Amp GPUI\Amp GPUI.lnk"
$desktop = "$([Environment]::GetFolderPath('Desktop'))\Amp GPUI.lnk"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{5B7C2E1A-8D34-4F6E-9A12-6E0F3C7D4B21}_is1"
$tone = "$env:APPDATA\amp-gpui\data\library\kept-by-uninstall.txt"

function Check($condition, $message) {
    if (-not $condition) { throw "FAILED: $message" }
    Write-Host "ok: $message"
}

function Install($path) {
    $process = Start-Process $path -Wait -PassThru -ArgumentList `
        "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/TASKS=desktopicon"
    Check ($process.ExitCode -eq 0) "$path exits cleanly"
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Shell {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    public static extern uint ExtractIconEx(string file, int index, IntPtr large, IntPtr small, uint count);
}
"@

Install $OldInstaller
Check ((Get-ItemProperty $uninstallKey).DisplayVersion -eq "0.0.1") "old version is installed"

Install $Installer
Check ((Get-ItemProperty $uninstallKey).DisplayVersion -eq $Version) "upgrade changes the version to $Version"
Check (Test-Path $exe) "app is in the per-user folder"
Check (Test-Path "$appDir\LICENSE.txt") "license is installed"
Check (Test-Path "$appDir\THIRD-PARTY-LICENSES.html") "third-party licenses are installed"
Check (Test-Path $startMenu) "Start menu shortcut exists"
Check (Test-Path $desktop) "desktop shortcut exists"
Check ([Shell]::ExtractIconEx($exe, -1, [IntPtr]::Zero, [IntPtr]::Zero, 0) -ge 1) "app has an icon"

# Offset 0x3C of the file points at the PE header, and the subsystem is 92 bytes
# past it. 2 means a windowed app, which opens no console.
$bytes = [IO.File]::ReadAllBytes($exe)
$header = [BitConverter]::ToInt32($bytes, 0x3C)
Check ([BitConverter]::ToUInt16($bytes, $header + 92) -eq 2) "app opens no console window"

$app = Start-Process $exe -PassThru
Start-Sleep -Seconds 10
Check (-not $app.HasExited) "app is still running after 10 seconds"
Stop-Process -Id $app.Id -Force

New-Item -ItemType Directory -Force (Split-Path $tone) | Out-Null
Set-Content $tone "tone"
$uninstaller = (Get-ItemProperty $uninstallKey).UninstallString.Trim('"')
Start-Process $uninstaller -Wait -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"

# The uninstaller hands off to a temporary copy of itself, so wait for the files.
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path $exe) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 1 }

Check (-not (Test-Path $exe)) "uninstall removes the app"
Check (-not (Test-Path $startMenu)) "uninstall removes the Start menu shortcut"
Check (-not (Test-Path $desktop)) "uninstall removes the desktop shortcut"
Check (-not (Test-Path $uninstallKey)) "uninstall removes the Apps list entry"
Check (Test-Path $tone) "uninstall keeps the tone library"
