param(
  [Parameter(Mandatory=$true)][string]$Installer,
  [string]$ConfigPath,
  [switch]$KeepInstalled
)
$ErrorActionPreference = 'Stop'
$serviceName = 'NetTimeNext'
$existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($existing) { throw 'The observer service already exists; this fresh-install test will not replace it.' }
$dataRoot = Join-Path $env:ProgramData 'NetTimeNext'
$configFile = Join-Path $dataRoot 'settings.json'
if (Test-Path -LiteralPath $configFile) { throw 'Existing observer settings found; refusing to overwrite them for testing.' }
$oldServices = @(Get-CimInstance Win32_Service | Where-Object { $_.Name -match '(?i)nettime' -and $_.Name -ne $serviceName } | Select-Object Name,State,ProcessId)
New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null
if ($ConfigPath) {
    Copy-Item -LiteralPath ([System.IO.Path]::GetFullPath($ConfigPath)) -Destination $configFile
} else {
    '{"servers":[{"host":"test.invalid","port":123,"security":"nts_required"}],"poll_seconds":64,"timeout_seconds":1,"retention_days":7,"max_distance_ms":1000}' | Set-Content -LiteralPath $configFile -Encoding utf8
}
$setupPath = [System.IO.Path]::GetFullPath($Installer)
$appRoot = Join-Path $env:ProgramFiles 'NetTime Next'
$helper = Join-Path $appRoot 'Service\NetTimeNextService.exe'
$logPath = Join-Path (Split-Path $setupPath) 'installer-test.log'
$startedUtc = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$keep = $false
try {
    $process = Start-Process -FilePath $setupPath -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/LOG=`"$logPath`"") -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed ($($process.ExitCode)); see $logPath" }
    $svc = Get-Service -Name $serviceName
    $svc.WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    $health = $null
    for ($attempt=0; $attempt -lt 30; $attempt++) {
        $health = & $helper health | ConvertFrom-Json
        if ($health.latest_decision_utc -ge $startedUtc -and $health.servers -gt 0) { break }
        Start-Sleep -Seconds 1
    }
    if (-not $health -or $health.latest_decision_utc -lt $startedUtc -or $health.servers -lt 1) { throw 'Service did not record a fresh measurement cycle.' }
    Write-Output ('Fresh service evidence: ' + ($health | ConvertTo-Json -Compress))
    & $helper stop
    if ($LASTEXITCODE -ne 0) { throw 'Service stop failed' }
    (Get-Service -Name $serviceName).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
    & $helper start
    if ($LASTEXITCODE -ne 0) { throw 'Service restart failed' }
    (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    $configHash = (Get-FileHash -LiteralPath $configFile -Algorithm SHA256).Hash
    $upgrade = Start-Process -FilePath $setupPath -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART') -WindowStyle Hidden -Wait -PassThru
    if ($upgrade.ExitCode -ne 0) { throw 'Upgrade/reinstall failed' }
    (Get-Service -Name $serviceName).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
    if ((Get-FileHash -LiteralPath $configFile -Algorithm SHA256).Hash -ne $configHash) { throw 'Upgrade changed machine settings' }
    foreach ($old in $oldServices) {
        $current = Get-CimInstance Win32_Service -Filter "Name='$($old.Name)'"
        if ($current.State -ne $old.State -or $current.ProcessId -ne $old.ProcessId) {
            throw "Existing time service changed unexpectedly: $($old.Name)"
        }
    }
    $currentService = Get-CimInstance Win32_Service -Filter "Name='$serviceName'"
    if ($currentService.StartName -ne 'NT AUTHORITY\LocalService') { throw 'Unexpected service account' }
    if ($currentService.StartMode -ne 'Auto') { throw 'Service startup is not automatic' }
    $keep = $KeepInstalled.IsPresent
    Write-Output 'PASS: install, automatic startup, fresh evidence, stop/start, upgrade and original-service coexistence.'
} finally {
    if (-not $keep -and (Test-Path -LiteralPath (Join-Path $appRoot 'unins000.exe'))) {
        $uninstaller = Start-Process -FilePath (Join-Path $appRoot 'unins000.exe') -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART') -WindowStyle Hidden -Wait -PassThru
        if ($uninstaller.ExitCode -ne 0) { throw 'Uninstall failed' }
        if (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) { throw 'Service still registered after uninstall' }
        if (-not (Test-Path -LiteralPath (Join-Path $dataRoot 'data\history.sqlite3'))) { throw 'Uninstall removed retained history' }
        Write-Output 'PASS: uninstall removes the service and preserves history.'
    }
}
