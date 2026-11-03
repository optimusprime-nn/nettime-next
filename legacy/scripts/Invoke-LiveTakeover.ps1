param(
    [Parameter(Mandatory=$true)][string]$Installer,
    [Parameter(Mandatory=$true)][string]$Configuration,
    [Parameter(Mandatory=$true)][string]$StatusFile,
    [int]$ReadinessTimeoutSeconds = 600,
    [int]$FirstCorrectionTimeoutSeconds = 1200
)
$ErrorActionPreference = 'Stop'
$installerPath = [IO.Path]::GetFullPath($Installer)
$configurationPath = [IO.Path]::GetFullPath($Configuration)
$statusPath = [IO.Path]::GetFullPath($StatusFile)
$helperPath = Join-Path $env:ProgramFiles 'NetTime Next/Service/NetTimeNextService.exe'
$activated = $false
function Report([string]$Phase, [string]$Message, $Details = $null) {
    $record = @{phase=$Phase; message=$Message; utc=[DateTimeOffset]::UtcNow.ToString('o'); details=$Details}
    $temporary = $statusPath + '.tmp'
    [IO.File]::WriteAllText($temporary, ($record | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $statusPath -Force
}
function Command([string[]]$Arguments) {
    $result = & $helperPath @Arguments
    if ($LASTEXITCODE -ne 0) { throw ('Service helper failed: ' + ($Arguments -join ' ') + ': ' + ($result -join "`n")) }
    return $result
}
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    if (-not ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Windows administrator approval is required for installation and service ownership changes.'
    }
    if (-not (Test-Path -LiteralPath $installerPath) -or -not (Test-Path -LiteralPath $configurationPath)) {
        throw 'Installer or prepared configuration is missing.'
    }
    Report 'installing' 'Installing takeover-enabled build; existing clock controller is still running.'
    $setupLog = $statusPath + '.installer.log'
    $setup = Start-Process -FilePath $installerPath -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/FORCECLOSEAPPLICATIONS',"/LOG=`"$setupLog`"") -WindowStyle Hidden -Wait -PassThru
    if ($setup.ExitCode -ne 0) { throw "Installer failed ($($setup.ExitCode)); see $setupLog" }
    Report 'configuring' 'Applying source groups; original clock controllers remain running.'
    Command -Arguments @('apply-config','--file',$configurationPath) | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds($ReadinessTimeoutSeconds)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        $health = (Command -Arguments @('health')) | ConvertFrom-Json
        if ($health.service -ne 'running') { throw 'Observer service stopped while warming source filters.' }
        if ($health.synchronization.ready) { $ready = $true; break }
        Report 'warming' ('Original clock still controls time. ' + $health.synchronization.explanation) $health
        Start-Sleep -Seconds 5
    }
    if (-not $ready) { throw 'Fresh source agreement was not established; the original clock services were left in control.' }
    Report 'taking_over' 'Fresh source agreement confirmed; saving rollback state and switching clock ownership.' $health.synchronization
    $takeoverUtc = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0
    Command -Arguments @('takeover') | Out-Null
    $activated = $true
    $deadline = [DateTime]::UtcNow.AddSeconds($FirstCorrectionTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $health = (Command -Arguments @('health')) | ConvertFrom-Json
        if ($health.service -ne 'running' -or $health.mode -ne 'slew') { throw 'New clock controller is not running in slew mode.' }
        if ($health.last_slew -and $health.last_slew.utc -ge $takeoverUtc) {
            $services = Get-CimInstance Win32_Service | Where-Object { $_.Name -in @('NetTimeNext','NetTimeSvc','W32Time') } | Select-Object Name,State,StartMode,StartName
            Report 'complete' 'Takeover complete; a bounded clock adjustment and Windows rate readback are recorded.' @{health=$health; services=$services}
            exit 0
        }
        Report 'active_warming' 'NetTime Next owns the clock; verifying its first bounded correction after filter warmup.' $health
        Start-Sleep -Seconds 5
    }
    throw 'No verified correction within the validation window; restoring previous clock ownership.'
} catch {
    $failure = $_.Exception.Message
    if ($activated) {
        try { Command -Arguments @('rollback') | Out-Null; $failure += ' Previous clock services have been restored.' }
        catch { $failure += ' Rollback also needs attention: ' + $_.Exception.Message }
    }
    Report 'failed' $failure
    exit 1
}
