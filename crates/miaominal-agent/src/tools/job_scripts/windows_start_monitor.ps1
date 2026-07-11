$ErrorActionPreference='Stop'; $marker=[Environment]::ExpandEnvironmentVariables(@@MARKER@@)
$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'
$workingDirectory=[Environment]::GetEnvironmentVariable('MIAOMINAL_AGENT_JOB_CWD','Process')
Remove-Item Env:MIAOMINAL_AGENT_JOB_CWD -ErrorAction SilentlyContinue
if ([string]::IsNullOrWhiteSpace($workingDirectory)) { throw 'job working directory was not provided' }
$self=[Diagnostics.Process]::GetCurrentProcess()
$statusTmp=$marker+'.tmp-'+[Guid]::NewGuid().ToString('N')
function Publish-MiaominalPidMetadata([hashtable]$metadata) {
    $pidJson=$metadata | ConvertTo-Json -Compress
    $pidTmp=$pidFile+'.tmp-'+[Guid]::NewGuid().ToString('N')
    [IO.File]::WriteAllText($pidTmp,$pidJson,(New-Object Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $pidTmp -Destination $pidFile -Force
}
$monitorMetadata=@{pid=$self.Id;start_ticks=([string]$self.StartTime.ToUniversalTime().Ticks)}
Publish-MiaominalPidMetadata $monitorMetadata
$exitCode=1; $process=$null; $childStartTicks=$null; $outStream=$null; $errStream=$null; $caughtError=$null
try {
    $psi=[Diagnostics.ProcessStartInfo]::new(); $psi.FileName=@@PROGRAM@@; $psi.Arguments=@@ARGUMENTS@@
    $psi.WorkingDirectory=$workingDirectory; $psi.UseShellExecute=$false
    $psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true
    $process=[Diagnostics.Process]::new(); $process.StartInfo=$psi; $share=[IO.FileShare]::ReadWrite
    $outStream=[IO.File]::Open($out,[IO.FileMode]::Create,[IO.FileAccess]::Write,$share)
    $errStream=[IO.File]::Open($err,[IO.FileMode]::Create,[IO.FileAccess]::Write,$share)
    [void]$process.Start()
    for ($identityAttempt=0; $identityAttempt -lt 50 -and $null -eq $childStartTicks; $identityAttempt++) {
        try { $childStartTicks=[int64]$process.StartTime.ToUniversalTime().Ticks } catch { Start-Sleep -Milliseconds 10 }
    }
    if ($null -eq $childStartTicks) { throw 'failed to capture child process identity' }
    $monitorMetadata['child_pid']=$process.Id; $monitorMetadata['child_start_ticks']=[string]$childStartTicks
    Publish-MiaominalPidMetadata $monitorMetadata
    $stdoutTask=$process.StandardOutput.BaseStream.CopyToAsync($outStream)
    $stderrTask=$process.StandardError.BaseStream.CopyToAsync($errStream)
    $process.WaitForExit(); $stdoutTask.Wait(); $stderrTask.Wait(); $exitCode=[int]$process.ExitCode
} catch {
    $caughtError=$_ | Out-String
    if ($null -ne $process -and $null -ne $childStartTicks) {
        try {
            $child=Get-Process -Id $process.Id -ErrorAction Stop
            if ($child.StartTime.ToUniversalTime().Ticks -eq $childStartTicks) {
                $savedErrorActionPreference=$ErrorActionPreference
                try { $ErrorActionPreference='Continue'; & taskkill.exe /T /F /PID $child.Id *> $null } finally { $ErrorActionPreference=$savedErrorActionPreference }
                try { $child.WaitForExit(5000) *> $null } catch {}
                if (-not $child.HasExited) { try { $child.Kill(); $child.WaitForExit(5000) *> $null } catch {} }
            }
        } catch {}
    } elseif ($null -ne $process) {
        try { if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit(5000) *> $null } } catch {}
    }
} finally {
    if ($null -ne $outStream) { $outStream.Dispose() }
    if ($null -ne $errStream) { $errStream.Dispose() }
}
if ($caughtError) { [IO.File]::AppendAllText($err,$caughtError,(New-Object Text.UTF8Encoding($false))) }
[IO.File]::WriteAllText($statusTmp,[string]$exitCode,(New-Object Text.UTF8Encoding($false)))
Move-Item -LiteralPath $statusTmp -Destination $marker -Force
exit $exitCode
