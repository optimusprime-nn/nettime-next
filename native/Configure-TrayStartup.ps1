param([string]$DesktopPath = (Join-Path $env:ProgramFiles 'NetTime Next\NetTimeNextNative.exe'))
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $DesktopPath -PathType Leaf)) { throw "desktop is not installed: $DesktopPath" }
$startupFolder = [Environment]::GetFolderPath('Startup')
if (-not $startupFolder) { throw 'Windows did not provide the current user startup folder.' }
$shortcutPath = Join-Path $startupFolder 'NetTime Next.lnk'
$resolvedDesktop = (Resolve-Path -LiteralPath $DesktopPath).Path
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $resolvedDesktop
$shortcut.Arguments = '--tray'
$shortcut.WorkingDirectory = Split-Path -Parent $resolvedDesktop
$shortcut.IconLocation = $resolvedDesktop + ',0'
$shortcut.Description = 'NetTime Next desktop — start minimised in the notification area'
# The native desktop creates only the tray icon; the service starts separately.
$shortcut.WindowStyle = 1
$shortcut.Save()
$saved = $shell.CreateShortcut($shortcutPath)
if ($saved.WindowStyle -ne 1 -or $saved.TargetPath -ne $shortcut.TargetPath -or $saved.Arguments -ne $shortcut.Arguments) { throw 'The tray startup shortcut could not be verified.' }
[pscustomobject]@{ shortcut = $shortcutPath; target = $saved.TargetPath; window_style = $saved.WindowStyle }
