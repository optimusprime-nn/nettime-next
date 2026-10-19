param([Parameter(Mandatory=$true)][string]$Installer)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Installed lifecycle test is restricted to disposable GitHub runners.' }
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Requires a disposable GitHub-hosted runner.' }
$setup=Start-Process -FilePath (Resolve-Path $Installer) -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TASKS=observer' -WindowStyle Hidden -Wait -PassThru
if ($setup.ExitCode -ne 0) { throw 'Rust preview installation failed' }
$root=Join-Path $env:ProgramData 'NetTimeNextNativePreview'
$app=Join-Path $env:ProgramFiles 'NetTime Next Rust Preview'
$helper=Join-Path $app 'NetTimeNextNativeService.exe'
& $helper --stop-service
if ($LASTEXITCODE -ne 0) { throw 'Could not stop native observer before fixture configuration' }
$fixture=@{servers=@(@{host='ntp.invalid';security='nts_required';source_group='fixture'});poll_seconds=64;timeout_seconds=1;retention_days=7;max_distance_ms=1000;clock=@{mode='observe'}}
[IO.File]::WriteAllText((Join-Path $root 'settings.json'),($fixture|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
& $helper --start-service
if ($LASTEXITCODE -ne 0) { throw 'Native service start failed' }
$deadline=[DateTime]::UtcNow.AddSeconds(45)
while ([DateTime]::UtcNow -lt $deadline) {
    $service=Get-CimInstance Win32_Service -Filter "Name='NetTimeNextNativePreview'"
    if ($service.State -eq 'Running' -and (Test-Path (Join-Path $root 'data/history.sqlite3'))) { break }
    Start-Sleep -Seconds 1
}
if ($service.State -ne 'Running') { throw 'Native service did not run' }
if ($service.StartName -ne 'NT AUTHORITY\LocalService') { throw 'Unexpected native service identity' }
Start-Sleep -Seconds 3
$database=Join-Path $root 'data/history.sqlite3'
& (Join-Path $PSScriptRoot 'test-takeover.ps1') -Helper $helper -Root $root
foreach ($screen in @('--smoke-ui','--smoke-settings','--smoke-comparison','--smoke-collection','--smoke-clock','--smoke-service','--smoke-appearance')) {
    $arguments="$screen --software-rendering --config `"$(Join-Path $root 'settings.json')`" --database `"$database`""
    $desktop=Start-Process -FilePath (Join-Path $app 'NetTimeNextNative.exe') -ArgumentList $arguments -Wait -PassThru
    if ($desktop.ExitCode -ne 0) { throw "Installed native desktop failed: $screen" }
}
$trayReport=Join-Path $PSScriptRoot '..\dist\rust-tray-lifecycle.json'
$trayArguments="--software-rendering --test-tray-lifecycle `"$trayReport`" --config `"$(Join-Path $root 'settings.json')`" --database `"$database`""
$trayDesktop=Start-Process -FilePath (Join-Path $app 'NetTimeNextNative.exe') -ArgumentList $trayArguments -PassThru
if (-not $trayDesktop.WaitForExit(45000)) { Stop-Process -Id $trayDesktop.Id; throw 'Installed tray lifecycle did not exit.' }
if ($trayDesktop.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $trayReport)) { throw 'Installed tray lifecycle did not complete.' }
if (-not (Get-Content -LiteralPath $trayReport -Raw | ConvertFrom-Json).passed) { throw 'Installed tray lifecycle failed.' }
& $helper --export (Join-Path $env:RUNNER_TEMP 'native-export') --database $database
if ($LASTEXITCODE -ne 0) { throw 'Native export failed' }
& $helper --compare-export (Join-Path $env:RUNNER_TEMP 'native-comparison') --database $database --days 7
if ($LASTEXITCODE -ne 0) { throw 'Native comparison export failed' }
$uninstall=Start-Process -FilePath (Join-Path $app 'unins000.exe') -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw 'Native uninstall failed' }
if (Get-Service NetTimeNextNativePreview -ErrorAction SilentlyContinue) { throw 'Native service remains after uninstall' }
if (-not (Test-Path $database)) { throw 'Uninstall removed retained history' }
Write-Output 'Rust installed service, LocalService identity, desktop, export, uninstall and retained history passed.'
