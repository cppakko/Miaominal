$ErrorActionPreference='Stop'
$marker=[Environment]::ExpandEnvironmentVariables(@@MARKER@@)
$pidFile=$marker+'.pid'; $runner=$marker+'.runner.ps1'
$cwdEnvName='MIAOMINAL_AGENT_JOB_CWD'
$previousCwdEnv=[Environment]::GetEnvironmentVariable($cwdEnvName,'Process')
$monitorPid=$null; $monitorStartTicks=$null
function Remove-MiaominalLaunchArtifacts {
    Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),$pidFile,($marker+'.ctl.out'),($marker+'.ctl.err'),$runner) -Force -ErrorAction SilentlyContinue
    $root=Split-Path -Parent $marker; $leaf=Split-Path -Leaf $marker
    Get-ChildItem -LiteralPath $root -Filter ($leaf+'.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $root -Filter ($leaf+'.pid.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
}
function Stop-MiaominalLaunchedMonitor([int]$processId,[object]$expectedTicks) {
    if ($null -eq $expectedTicks -and (Test-Path -LiteralPath $pidFile -PathType Leaf)) {
        try {
            $metadata=Get-Content -LiteralPath $pidFile -Raw -ErrorAction Stop | ConvertFrom-Json
            if ([int]$metadata.pid -eq $processId) { $expectedTicks=[int64]$metadata.start_ticks }
        } catch {}
    }
    if ($null -eq $expectedTicks) { throw 'cannot validate monitor process identity before cleanup' }
    $process=$null; try { $process=Get-Process -Id $processId -ErrorAction Stop } catch { return }
    $actualTicks=[int64]$process.StartTime.ToUniversalTime().Ticks
    if ($actualTicks -ne ([int64]$expectedTicks)) { throw ('monitor process identity mismatch: expected '+$expectedTicks+', actual '+$actualTicks) }
    try {
        $savedErrorActionPreference=$ErrorActionPreference
        try { $ErrorActionPreference='Continue'; & taskkill.exe /T /F /PID $processId *> $null } finally { $ErrorActionPreference=$savedErrorActionPreference }
        try { $process.WaitForExit(5000) *> $null } catch {}
        if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit(5000) *> $null }
        if (-not $process.HasExited) { throw 'monitor process survived cleanup' }
    } catch { throw }
}
try {
    $requestedCwd=[Environment]::ExpandEnvironmentVariables(@@CWD@@)
    if ([IO.Path]::IsPathRooted($requestedCwd)) { $cwdPath=$requestedCwd } else { $cwdPath=Join-Path $env:USERPROFILE $requestedCwd }
    $cwdItem=Get-Item -LiteralPath $cwdPath -Force -ErrorAction Stop
    if (-not $cwdItem.PSIsContainer) { throw 'job working directory is not a directory' }
    $resolvedCwd=$cwdItem.FullName
    Remove-MiaominalLaunchArtifacts
    $powershell=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'; Add-Type -TypeDefinition @@DETACHED_LAUNCHER@@ -Language CSharp
    [IO.File]::WriteAllText($runner,@@MONITOR_SCRIPT@@,(New-Object Text.UTF8Encoding($true)))
    $monitorArgs='-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "'+$runner+'"'
    [Environment]::SetEnvironmentVariable($cwdEnvName,$resolvedCwd,'Process')
    $monitorPid=[MiaominalDetachedProcess]::Start($powershell,$monitorArgs,(Split-Path -Parent $runner)); $monitorStartTicks=[int64][MiaominalDetachedProcess]::LastStartTicks
    if ($monitorStartTicks -le 0) { throw 'detached launcher did not return monitor identity' }
    for ($i=0; $i -lt 1000 -and -not (Test-Path -LiteralPath $pidFile) -and -not (Test-Path -LiteralPath $marker); $i++) { Start-Sleep -Milliseconds 10 }
    if (-not (Test-Path -LiteralPath $pidFile) -and -not (Test-Path -LiteralPath $marker)) { throw 'job monitor failed to publish metadata' }
    Write-Output $marker
} catch {
    $launchError=$_; $cleanupFailure=$null
    if ($null -ne $monitorPid) { try { Stop-MiaominalLaunchedMonitor ([int]$monitorPid) $monitorStartTicks } catch { $cleanupFailure=$_ } }
    if ($null -ne $cleanupFailure) { throw ('job launch failed and monitor cleanup failed; artifacts were preserved for scavenging: '+($cleanupFailure | Out-String)+'; launch error: '+($launchError | Out-String)) }
    Remove-MiaominalLaunchArtifacts; Start-Sleep -Milliseconds 100; Remove-MiaominalLaunchArtifacts
    throw $launchError
} finally {
    [Environment]::SetEnvironmentVariable($cwdEnvName,$previousCwdEnv,'Process')
}
