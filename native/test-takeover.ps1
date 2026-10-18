param([Parameter(Mandatory=$true)][string]$Helper,[Parameter(Mandatory=$true)][string]$Root)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Clock lifecycle tests require a disposable GitHub-hosted runner.' }
function Invoke-Native([string[]]$Arguments) {
    $result = & $Helper @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Native command failed: $($Arguments[0])" }
    return $result
}
function Health { return ((Invoke-Native @('--health')) | ConvertFrom-Json) }
function Controllers {
    $states=@()
    foreach ($name in @('NetTimeSvc','W32Time','NetTimeNext')) {
        $svc=Get-CimInstance Win32_Service -Filter "Name='$name'"
        if ($svc) { $states+=@{name=$name;start=$svc.StartMode;running=($svc.State -eq 'Running')} }
    }
    return $states
}
function Restore-Controllers($states) {
    foreach ($state in $states) {
        $mode=switch ($state.start) {'Auto' {'Automatic'} 'Manual' {'Manual'} 'Disabled' {'Disabled'} default {throw 'Unexpected startup mode'}}
        Set-Service -Name $state.name -StartupType $mode
        if ($state.running) { Start-Service -Name $state.name } else { Stop-Service -Name $state.name -ErrorAction Stop }
    }
}
$saved=Controllers
$request=Join-Path $env:RUNNER_TEMP 'native-settings-request.json'
$config=Join-Path $Root 'settings.json'
$database=Join-Path $Root 'data/history.sqlite3'
try {
    $beforeHash=(Get-FileHash -LiteralPath $config -Algorithm SHA256).Hash
    $invalid=Get-Content -LiteralPath $config -Raw | ConvertFrom-Json
    $invalid.retention_days=1
    [IO.File]::WriteAllText($request,($invalid|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
    & $Helper --apply-config $request
    if ($LASTEXITCODE -eq 0) { throw 'Invalid settings were accepted' }
    if ((Get-FileHash -LiteralPath $config -Algorithm SHA256).Hash -ne $beforeHash) { throw 'Invalid settings modified configuration' }
    $valid=Get-Content -LiteralPath $config -Raw | ConvertFrom-Json
    $valid.clock.mode='slew'
    [IO.File]::WriteAllText($request,($valid|ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
    Invoke-Native @('--apply-config',$request) | Out-Null
    $before=Health
    if ($before.mode -ne 'observe' -or -not $before.service.running) { throw 'Settings apply enabled takeover or failed to restart observer' }
    $savedConfig=[IO.File]::ReadAllBytes($config)
    # Administrative SCM fixture only: no valid NTP samples are inserted. On
    # restart all native filters are empty, so this proof cannot drive a slew.
    @'
import sqlite3,json,time,sys,pathlib
now=time.time()
instance=json.loads((pathlib.Path(sys.argv[1]).parent/'service-instance.json').read_text())['id']
proof={'utc':now,'collector_instance':instance,'selected':'CI transaction fixture only','policy':'ci-administrative-lifecycle','synchronization':{'ready':True,'sample_utc':now,'offset_ms':1,'explanation':'SCM transaction fixture; not a clock measurement'}}
with sqlite3.connect(sys.argv[1],timeout=10) as db:
    cycle=db.execute('SELECT COALESCE(MAX(cycle),0)+1 FROM decisions').fetchone()[0]
    db.execute('INSERT INTO decisions(cycle,utc,selected,confidence,payload)VALUES(?,?,?,?,?)',(cycle,now,proof['selected'],'CI only',json.dumps(proof)))
'@ | python - $database
    if ($LASTEXITCODE -ne 0) { throw 'Cannot write isolated CI transaction fixture' }
    Invoke-Native @('--takeover') | Out-Null
    $active=Health
    if ($active.mode -ne 'slew' -or $active.ownership.phase -ne 'active' -or -not $active.service.running) { throw 'Native service did not acquire journaled ownership' }
    if ($active.service.account -ne 'NT SERVICE\NetTimeNextNativePreview' -or -not $active.clock_privilege) { throw 'Native service identity/privilege not confined to virtual account' }
    foreach ($state in (Controllers)) { if ($state.running -or $state.start -ne 'Disabled') { throw 'Competing clock controller remains enabled' } }
    @'
import sqlite3,sys
with sqlite3.connect(sys.argv[1],timeout=10) as db:
    assert db.execute("SELECT COUNT(*) FROM events WHERE kind='clock_correction' AND json_extract(payload,'$.action')='slew'").fetchone()[0]==0,'A lifecycle fixture drove a clock slew'
'@ | python - $database
    if ($LASTEXITCODE -ne 0) { throw 'Administrative fixture affected correction' }
    $crashed=Get-CimInstance Win32_Service -Filter "Name='NetTimeNextNativePreview'"
    Stop-Process -Id $crashed.ProcessId -Force
    $recoveryDeadline=[DateTime]::UtcNow.AddSeconds(20)
    do { Start-Sleep -Milliseconds 250; $recovered=Get-CimInstance Win32_Service -Filter "Name='NetTimeNextNativePreview'" } while (($recovered.State -ne 'Running' -or $recovered.ProcessId -eq $crashed.ProcessId) -and [DateTime]::UtcNow -lt $recoveryDeadline)
    if ($recovered.State -ne 'Running' -or $recovered.ProcessId -eq $crashed.ProcessId) { throw 'SCM crash recovery failed' }
    # Revoke ownership while the collector is running; independent watchdog must
    # restore neutral and stop the service without waiting for another poll.
    $journal=Join-Path $Root 'clock-ownership.json'
    $state=Get-Content -LiteralPath $journal -Raw | ConvertFrom-Json
    $state.phase='ci_guard_revoked'
    [IO.File]::WriteAllText($journal,($state|ConvertTo-Json -Depth 30),[Text.UTF8Encoding]::new($false))
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    do { Start-Sleep -Milliseconds 250; $svc=Get-Service NetTimeNextNativePreview } while ($svc.Status -ne 'Stopped' -and [DateTime]::UtcNow -lt $deadline)
    if ($svc.Status -ne 'Stopped') { throw 'Clock ownership watchdog did not stop the service' }
    Invoke-Native @('--rollback') | Out-Null
    $after=Health
    if ($after.mode -ne 'observe' -or $after.ownership.phase -ne 'rolled_back' -or $after.service.account -ne $before.service.account -or $after.clock_privilege -ne $before.clock_privilege) { throw 'Rollback did not restore mode, account and privilege' }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($config)) -ne [Convert]::ToBase64String($savedConfig)) { throw 'Rollback did not preserve exact configuration bytes' }
    foreach ($old in $saved) {
        $actual=Get-CimInstance Win32_Service -Filter "Name='$($old.name)'"
        if ($actual.StartMode -ne $old.start -or ($actual.State -eq 'Running') -ne $old.running) { throw "Clock service state not restored: $($old.name)" }
    }
    Invoke-Native @('--stop-service') | Out-Null
    foreach ($old in $saved) { Stop-Service $old.name; Set-Service $old.name -StartupType Disabled }
    Invoke-Native @('--ci-clock-response',(Join-Path $PSScriptRoot '../dist/rust-clock-response.json')) | Write-Output
    Write-Output 'PASS: native settings, takeover, virtual identity/privilege, independent watchdog, exact rollback and physical actuator response.'
} finally {
    & $Helper --rollback
    if ($LASTEXITCODE -ne 0) { throw 'CI rollback incomplete' }
    Restore-Controllers $saved
}
