$marker=[Environment]::ExpandEnvironmentVariables(@@MARKER@@)
$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'
$ctlOut=$marker+'.ctl.out'; $ctlErr=$marker+'.ctl.err'; $runner=$marker+'.runner.ps1'
$artifacts=@($marker,$out,$err,$pidFile,$ctlOut,$ctlErr,$runner)
if (-not ($artifacts | Where-Object { Test-Path -LiteralPath $_ })) { Write-Output 'not_found'; exit 0 }
if (Test-Path -LiteralPath $marker -PathType Leaf) {
    Remove-Item -LiteralPath @($out,$err,$pidFile,$ctlOut,$ctlErr,$runner) -Force -ErrorAction SilentlyContinue
    Write-Output 'already_finished'; exit 0
}
$valid=$false; $metadata=$null
if (Test-Path -LiteralPath $pidFile -PathType Leaf) {
    try {
        $stream=[IO.File]::Open($pidFile,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
        $count=[int][Math]::Min(4096,$stream.Length); $bytes=[byte[]]::new($count)
        $read=$stream.Read($bytes,0,$count); $stream.Dispose()
        $metadata=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$read) | ConvertFrom-Json
        $process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop
        $valid=$process.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.start_ticks)
    } catch { $valid=$false }
}
$childValid=$false; $childProcess=$null
if ($valid -and $null -ne $metadata.child_pid -and $null -ne $metadata.child_start_ticks) {
    try {
        $childProcess=Get-Process -Id ([int]$metadata.child_pid) -ErrorAction Stop
        $childValid=$childProcess.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.child_start_ticks)
    } catch { $childValid=$false }
}
if ($valid) {
    $targetProcessId=[int]$metadata.pid; $taskkillOutput=(& taskkill.exe /T /F /PID $targetProcessId 2>&1 | Out-String)
    $killExitCode=$LASTEXITCODE; $stopped=$process.WaitForExit(1000)
    if (-not $stopped) {
        if ($childValid -and -not $childProcess.HasExited) { try { $childProcess.Kill(); $childProcess.WaitForExit(5000) *> $null } catch {} }
        try { if (-not $process.HasExited) { $process.Kill() }; $stopped=$process.WaitForExit(5000) } catch { $stopped=$false }
    }
    if (-not $stopped -or ($childValid -and -not $childProcess.HasExited)) { throw ('failed to stop job process tree; taskkill exit code '+$killExitCode+': '+$taskkillOutput) }
}
Remove-Item -LiteralPath @($out,$err,$pidFile,$ctlOut,$ctlErr,$runner) -Force -ErrorAction SilentlyContinue
$statusTmp=$marker+'.tmp-'+[Guid]::NewGuid().ToString('N')
[IO.File]::WriteAllText($statusTmp,'stopped',(New-Object Text.UTF8Encoding($false)))
Move-Item -LiteralPath $statusTmp -Destination $marker -Force
Write-Output 'stopped'
