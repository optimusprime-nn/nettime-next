param([string]$DesktopPath = (Join-Path $env:ProgramFiles 'NetTime Next Rust Preview\NetTimeNextNative.exe'))
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $DesktopPath -PathType Leaf)) { throw "Rust desktop is not installed: $DesktopPath" }
$startupFolder = [Environment]::GetFolderPath('Startup')
if (-not $startupFolder) { throw 'Windows did not provide the current user startup folder.' }
$shortcutPath = Join-Path $startupFolder 'NetTime Next Rust.lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = (Resolve-Path -LiteralPath $DesktopPath).Path
$shortcut.WorkingDirectory = Split-Path -Parent $shortcut.TargetPath
$shortcut.IconLocation = $shortcut.TargetPath + ',0'
$shortcut.Description = 'NetTime Next Rust desktop — start minimised in the notification area'
# Windows launches the desktop minimised; its existing tray handler hides it.
# The automatic Windows service starts separately, before interactive sign-in.
$shortcut.WindowStyle = 7
$shortcut.Save()
$saved = $shell.CreateShortcut($shortcutPath)
if ($saved.WindowStyle -ne 7 -or $saved.TargetPath -ne $shortcut.TargetPath) { throw 'The tray startup shortcut could not be verified.' }
[pscustomobject]@{ shortcut = $shortcutPath; target = $saved.TargetPath; window_style = $saved.WindowStyle }
