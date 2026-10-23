param([Parameter(Mandatory=$true)][string]$DesktopPath)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $DesktopPath -PathType Leaf)) { exit 0 }
$desktop = Start-Process -FilePath $DesktopPath -WindowStyle Minimized -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while ([DateTime]::UtcNow -lt $deadline) {
    $desktop.Refresh()
    if ($desktop.HasExited) { throw 'The desktop exited during sign-in startup.' }
    if ($desktop.MainWindowTitle -like 'NetTime Next*') {
        # Use the application's normal X-to-tray handler once its window exists.
        # Only this newly launched desktop process is addressed; no service calls.
        if (-not $desktop.CloseMainWindow()) { throw 'The desktop could not be hidden in its tray.' }
        exit 0
    }
    Start-Sleep -Milliseconds 100
}
throw 'The desktop did not create a window before the startup timeout.'
