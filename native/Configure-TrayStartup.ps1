param([string]$DesktopPath = (Join-Path $env:ProgramFiles 'NetTime Next Rust Preview\NetTimeNextNative.exe'))
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $DesktopPath -PathType Leaf)) { throw "Rust desktop is not installed: $DesktopPath" }
$startupFolder = [Environment]::GetFolderPath('Startup')
if (-not $startupFolder) { throw 'Windows did not provide the current user startup folder.' }
$shortcutPath = Join-Path $startupFolder 'NetTime Next Rust.lnk'
$launcherSource = Join-Path $PSScriptRoot 'Start-DesktopTray.ps1'
$launcherFolder = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'NetTimeNextNativePreview'
New-Item -ItemType Directory -Path $launcherFolder -Force | Out-Null
$launcherPath = Join-Path $launcherFolder 'Start-DesktopTray.ps1'
Copy-Item -LiteralPath $launcherSource -Destination $launcherPath -Force
$resolvedDesktop = (Resolve-Path -LiteralPath $DesktopPath).Path
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$shortcut.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy RemoteSigned -WindowStyle Hidden -File "' + $launcherPath + '" -DesktopPath "' + $resolvedDesktop + '"'
$shortcut.WorkingDirectory = Split-Path -Parent $resolvedDesktop
$shortcut.IconLocation = $resolvedDesktop + ',0'
$shortcut.Description = 'NetTime Next Rust desktop — start minimised in the notification area'
# The launcher starts the desktop minimised, then requests its X-to-tray action.
# The automatic Windows service starts separately, before interactive sign-in.
$shortcut.WindowStyle = 7
$shortcut.Save()
$saved = $shell.CreateShortcut($shortcutPath)
if ($saved.WindowStyle -ne 7 -or $saved.TargetPath -ne $shortcut.TargetPath -or $saved.Arguments -ne $shortcut.Arguments) { throw 'The tray startup shortcut could not be verified.' }
[pscustomobject]@{ shortcut = $shortcutPath; target = $saved.TargetPath; window_style = $saved.WindowStyle }
