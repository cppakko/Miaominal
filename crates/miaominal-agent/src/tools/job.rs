use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, JobPollResult, JobStatus};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use base64::Engine as _;
use miaominal_core::profile::ShellType;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct StartJobArgs {
    pub command: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PollJobArgs {
    pub job_id: AgentJobId,
}

#[derive(Debug, Deserialize)]
pub struct StopJobArgs {
    pub job_id: AgentJobId,
}

const DEFAULT_POLL_AFTER_MS: u64 = 1_000;
const STALE_JOB_HOURS: u64 = 24;
const WINDOWS_CMD_MAX_COMMAND_BYTES: usize = 8_191;

/// Build the background job command for the given shell type.
///
/// POSIX/Fish use a detached `nohup` wrapper. Windows uses a short-lived
/// PowerShell launcher which starts a separate monitor process. The monitor
/// owns the user process, redirects its output to files, and atomically writes
/// the final exit status. Unlike `Start-Job`, this state survives the launcher
/// PowerShell session exiting.
fn make_start_job_command(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let cwd_q = shell_quote(cwd, shell_type);
            let cmd_q = shell_quote(user_command, shell_type);
            let marker_q = shell_quote(marker, shell_type);
            format!(
                "cd \"$HOME\" && cd {cwd_q} && nohup sh -lc {cmd_q} >{marker_q}.out 2>{marker_q}.err; printf $? >{marker_q}"
            )
        }
        ShellType::PowerShell => super::windows::powershell_compressed_command(
            &make_windows_start_script(shell_type, cwd, user_command, marker),
        ),
        ShellType::Cmd => super::windows::powershell_compressed_command_for_cmd(
            &make_windows_start_script(shell_type, cwd, user_command, marker),
        ),
    }
}

fn make_start_job_launch(shell_type: ShellType, job_command: &str, marker: &str) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let marker_q = shell_quote(marker, shell_type);
            format!("({job_command}) >/dev/null 2>&1 & printf '%s' {marker_q}")
        }
        ShellType::PowerShell | ShellType::Cmd => job_command.to_string(),
    }
}

fn make_windows_start_script(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    let marker_q = shell_quote(marker, ShellType::PowerShell);
    let requested_cwd_q = shell_quote(cwd, ShellType::PowerShell);
    let (program, child_arguments) = windows_child_command(shell_type, user_command);
    let program_q = shell_quote(program, ShellType::PowerShell);
    let child_arguments_q = shell_quote(&child_arguments, ShellType::PowerShell);

    let monitor_script = format!(
        concat!(
            "$ErrorActionPreference='Stop'; ",
            "$marker=[Environment]::ExpandEnvironmentVariables({marker}); ",
            "$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'; ",
            "$workingDirectory=[Environment]::GetEnvironmentVariable('MIAOMINAL_AGENT_JOB_CWD','Process'); ",
            "Remove-Item Env:MIAOMINAL_AGENT_JOB_CWD -ErrorAction SilentlyContinue; ",
            "if ([string]::IsNullOrWhiteSpace($workingDirectory)) {{ throw 'job working directory was not provided' }}; ",
            "$self=[Diagnostics.Process]::GetCurrentProcess(); ",
            "$statusTmp=$marker+'.tmp-'+[Guid]::NewGuid().ToString('N'); ",
            "function Publish-MiaominalPidMetadata([hashtable]$metadata) {{ ",
            "$pidJson=$metadata | ConvertTo-Json -Compress; $pidTmp=$pidFile+'.tmp-'+[Guid]::NewGuid().ToString('N'); ",
            "[IO.File]::WriteAllText($pidTmp,$pidJson,(New-Object Text.UTF8Encoding($false))); ",
            "Move-Item -LiteralPath $pidTmp -Destination $pidFile -Force ",
            "}}; ",
            "$monitorMetadata=@{{pid=$self.Id;start_ticks=([string]$self.StartTime.ToUniversalTime().Ticks)}}; ",
            "Publish-MiaominalPidMetadata $monitorMetadata; ",
            "$exitCode=1; $process=$null; $childStartTicks=$null; $outStream=$null; $errStream=$null; $caughtError=$null; ",
            "try {{ ",
            "$psi=[Diagnostics.ProcessStartInfo]::new(); $psi.FileName={program}; $psi.Arguments={arguments}; ",
            "$psi.WorkingDirectory=$workingDirectory; $psi.UseShellExecute=$false; ",
            "$psi.RedirectStandardOutput=$true; $psi.RedirectStandardError=$true; ",
            "$process=[Diagnostics.Process]::new(); $process.StartInfo=$psi; $share=[IO.FileShare]::ReadWrite; ",
            "$outStream=[IO.File]::Open($out,[IO.FileMode]::Create,[IO.FileAccess]::Write,$share); ",
            "$errStream=[IO.File]::Open($err,[IO.FileMode]::Create,[IO.FileAccess]::Write,$share); ",
            "[void]$process.Start(); ",
            "for ($identityAttempt=0; $identityAttempt -lt 50 -and $null -eq $childStartTicks; $identityAttempt++) {{ ",
            "try {{ $childStartTicks=[int64]$process.StartTime.ToUniversalTime().Ticks }} catch {{ Start-Sleep -Milliseconds 10 }} ",
            "}}; ",
            "if ($null -eq $childStartTicks) {{ throw 'failed to capture child process identity' }}; ",
            "$monitorMetadata['child_pid']=$process.Id; $monitorMetadata['child_start_ticks']=[string]$childStartTicks; ",
            "Publish-MiaominalPidMetadata $monitorMetadata; ",
            "$stdoutTask=$process.StandardOutput.BaseStream.CopyToAsync($outStream); ",
            "$stderrTask=$process.StandardError.BaseStream.CopyToAsync($errStream); ",
            "$process.WaitForExit(); $stdoutTask.Wait(); $stderrTask.Wait(); $exitCode=[int]$process.ExitCode ",
            "}} catch {{ ",
            "$caughtError=$_ | Out-String; ",
            "if ($null -ne $process -and $null -ne $childStartTicks) {{ ",
            "try {{ $child=Get-Process -Id $process.Id -ErrorAction Stop; ",
            "if ($child.StartTime.ToUniversalTime().Ticks -eq $childStartTicks) {{ ",
            "$savedErrorActionPreference=$ErrorActionPreference; ",
            "try {{ $ErrorActionPreference='Continue'; & taskkill.exe /T /F /PID $child.Id *> $null }} finally {{ $ErrorActionPreference=$savedErrorActionPreference }}; ",
            "try {{ $child.WaitForExit(5000) *> $null }} catch {{}}; ",
            "if (-not $child.HasExited) {{ try {{ $child.Kill(); $child.WaitForExit(5000) *> $null }} catch {{}} }} ",
            "}} }} catch {{}} ",
            "}} elseif ($null -ne $process) {{ ",
            "try {{ if (-not $process.HasExited) {{ $process.Kill(); $process.WaitForExit(5000) *> $null }} }} catch {{}} ",
            "}} ",
            "}} finally {{ ",
            "if ($null -ne $outStream) {{ $outStream.Dispose() }}; if ($null -ne $errStream) {{ $errStream.Dispose() }} ",
            "}}; ",
            "if ($caughtError) {{ [IO.File]::AppendAllText($err,$caughtError,(New-Object Text.UTF8Encoding($false))) }}; ",
            "[IO.File]::WriteAllText($statusTmp,[string]$exitCode,(New-Object Text.UTF8Encoding($false))); ",
            "Move-Item -LiteralPath $statusTmp -Destination $marker -Force; ",
            "exit $exitCode"
        ),
        marker = marker_q,
        program = program_q,
        arguments = child_arguments_q,
    );
    let monitor_script_q = shell_quote(&monitor_script, ShellType::PowerShell);
    let detached_launcher_q =
        shell_quote(windows_detached_launcher_source(), ShellType::PowerShell);

    format!(
        concat!(
            "$ErrorActionPreference='Stop'; ",
            "$marker=[Environment]::ExpandEnvironmentVariables({marker}); ",
            "$pidFile=$marker+'.pid'; $runner=$marker+'.runner.ps1'; ",
            "$cwdEnvName='MIAOMINAL_AGENT_JOB_CWD'; ",
            "$previousCwdEnv=[Environment]::GetEnvironmentVariable($cwdEnvName,'Process'); ",
            "$monitorPid=$null; $monitorStartTicks=$null; ",
            "function Remove-MiaominalLaunchArtifacts {{ ",
            "Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),$pidFile,($marker+'.ctl.out'),($marker+'.ctl.err'),$runner) ",
            "-Force -ErrorAction SilentlyContinue; ",
            "$root=Split-Path -Parent $marker; $leaf=Split-Path -Leaf $marker; ",
            "Get-ChildItem -LiteralPath $root -Filter ($leaf+'.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue; ",
            "Get-ChildItem -LiteralPath $root -Filter ($leaf+'.pid.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue ",
            "}}; ",
            "function Stop-MiaominalLaunchedMonitor([int]$processId,[object]$expectedTicks) {{ ",
            "if ($null -eq $expectedTicks -and (Test-Path -LiteralPath $pidFile -PathType Leaf)) {{ ",
            "try {{ $metadata=Get-Content -LiteralPath $pidFile -Raw -ErrorAction Stop | ConvertFrom-Json; ",
            "if ([int]$metadata.pid -eq $processId) {{ $expectedTicks=[int64]$metadata.start_ticks }} }} catch {{}} ",
            "}}; ",
            "if ($null -eq $expectedTicks) {{ throw 'cannot validate monitor process identity before cleanup' }}; ",
            "$process=$null; try {{ $process=Get-Process -Id $processId -ErrorAction Stop }} catch {{ return }}; ",
            "$actualTicks=[int64]$process.StartTime.ToUniversalTime().Ticks; ",
            "if ($actualTicks -ne ([int64]$expectedTicks)) {{ throw ('monitor process identity mismatch: expected '+$expectedTicks+', actual '+$actualTicks) }}; ",
            "try {{ ",
            "$savedErrorActionPreference=$ErrorActionPreference; ",
            "try {{ $ErrorActionPreference='Continue'; & taskkill.exe /T /F /PID $processId *> $null }} finally {{ $ErrorActionPreference=$savedErrorActionPreference }}; ",
            "try {{ $process.WaitForExit(5000) *> $null }} catch {{}}; ",
            "if (-not $process.HasExited) {{ $process.Kill(); $process.WaitForExit(5000) *> $null }}; ",
            "if (-not $process.HasExited) {{ throw 'monitor process survived cleanup' }} ",
            "}} catch {{ throw }} ",
            "}}; ",
            "try {{ ",
            "$requestedCwd=[Environment]::ExpandEnvironmentVariables({cwd}); ",
            "if ([IO.Path]::IsPathRooted($requestedCwd)) {{ $cwdPath=$requestedCwd }} else {{ $cwdPath=Join-Path $env:USERPROFILE $requestedCwd }}; ",
            "$cwdItem=Get-Item -LiteralPath $cwdPath -Force -ErrorAction Stop; ",
            "if (-not $cwdItem.PSIsContainer) {{ throw 'job working directory is not a directory' }}; ",
            "$resolvedCwd=$cwdItem.FullName; ",
            "Remove-MiaominalLaunchArtifacts; ",
            "$powershell=Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\powershell.exe'; ",
            "Add-Type -TypeDefinition {detached_launcher} -Language CSharp; ",
            "[IO.File]::WriteAllText($runner,{monitor_script},(New-Object Text.UTF8Encoding($true))); ",
            "$monitorArgs='-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"'+$runner+'\"'; ",
            "[Environment]::SetEnvironmentVariable($cwdEnvName,$resolvedCwd,'Process'); ",
            "$monitorPid=[MiaominalDetachedProcess]::Start($powershell,$monitorArgs,(Split-Path -Parent $runner)); ",
            "$monitorStartTicks=[int64][MiaominalDetachedProcess]::LastStartTicks; ",
            "if ($monitorStartTicks -le 0) {{ throw 'detached launcher did not return monitor identity' }}; ",
            "for ($i=0; $i -lt 1000 -and -not (Test-Path -LiteralPath $pidFile) -and -not (Test-Path -LiteralPath $marker); $i++) {{ ",
            "Start-Sleep -Milliseconds 10 ",
            "}}; ",
            "if (-not (Test-Path -LiteralPath $pidFile) -and -not (Test-Path -LiteralPath $marker)) {{ ",
            "throw 'job monitor failed to publish metadata' ",
            "}}; ",
            "Write-Output $marker ",
            "}} catch {{ ",
            "$launchError=$_; $cleanupFailure=$null; ",
            "if ($null -ne $monitorPid) {{ try {{ Stop-MiaominalLaunchedMonitor ([int]$monitorPid) $monitorStartTicks }} catch {{ $cleanupFailure=$_ }} }}; ",
            "if ($null -ne $cleanupFailure) {{ throw ('job launch failed and monitor cleanup failed; artifacts were preserved for scavenging: '+($cleanupFailure | Out-String)+'; launch error: '+($launchError | Out-String)) }}; ",
            "Remove-MiaominalLaunchArtifacts; Start-Sleep -Milliseconds 100; Remove-MiaominalLaunchArtifacts; ",
            "throw $launchError ",
            "}} finally {{ ",
            "[Environment]::SetEnvironmentVariable($cwdEnvName,$previousCwdEnv,'Process') ",
            "}}"
        ),
        marker = marker_q,
        cwd = requested_cwd_q,
        monitor_script = monitor_script_q,
        detached_launcher = detached_launcher_q,
    )
}

fn windows_detached_launcher_source() -> &'static str {
    r#"using System;using System.Runtime.InteropServices;using System.Text;
public static class MiaominalDetachedProcess{
[StructLayout(LayoutKind.Sequential)]struct S{public int cb;public IntPtr r,d,t;public int x,y,xs,ys,xc,yc,fa,fl;public short sw,cr;public IntPtr rr,i,o,e;}
[StructLayout(LayoutKind.Sequential)]struct P{public IntPtr p,t;public int id,tid;}
[DllImport("kernel32",SetLastError=true,CharSet=CharSet.Unicode)]static extern bool CreateProcessW(string a,StringBuilder c,IntPtr pa,IntPtr ta,bool h,uint f,IntPtr e,string d,ref S s,out P p);
[DllImport("kernel32",SetLastError=true)]static extern bool GetProcessTimes(IntPtr h,out long c,out long x,out long k,out long u);
[DllImport("kernel32")]static extern bool TerminateProcess(IntPtr h,uint c);
[DllImport("kernel32",SetLastError=true)]static extern uint ResumeThread(IntPtr h);
[DllImport("kernel32")]static extern uint WaitForSingleObject(IntPtr h,uint m);
[DllImport("kernel32")]static extern bool CloseHandle(IntPtr h);
public static long LastStartTicks;
public static int Start(string a,string g,string d){S s=new S();s.cb=Marshal.SizeOf(typeof(S));P p;uint f=0x08000204;StringBuilder c=new StringBuilder("\""+a+"\" "+g);bool ok=CreateProcessW(a,c,IntPtr.Zero,IntPtr.Zero,false,f|0x01000000,IntPtr.Zero,d,ref s,out p);if(!ok){c=new StringBuilder("\""+a+"\" "+g);ok=CreateProcessW(a,c,IntPtr.Zero,IntPtr.Zero,false,f,IntPtr.Zero,d,ref s,out p);}if(!ok)throw new Exception("CreateProcess failed: "+Marshal.GetLastWin32Error());try{long created,exited,kernel,user;if(!GetProcessTimes(p.p,out created,out exited,out kernel,out user))throw new Exception("GetProcessTimes failed: "+Marshal.GetLastWin32Error());LastStartTicks=DateTime.FromFileTimeUtc(created).Ticks;if(ResumeThread(p.t)==0xffffffff)throw new Exception("ResumeThread failed: "+Marshal.GetLastWin32Error());}catch{TerminateProcess(p.p,1);WaitForSingleObject(p.p,5000);CloseHandle(p.t);CloseHandle(p.p);throw;}CloseHandle(p.t);CloseHandle(p.p);return p.id;}}
"#
}

fn windows_child_command(shell_type: ShellType, user_command: &str) -> (&'static str, String) {
    match shell_type {
        ShellType::PowerShell => {
            let command_q = shell_quote(user_command, ShellType::PowerShell);
            let script = format!(
                concat!(
                    "$ErrorActionPreference='Stop'; ",
                    "$global:LASTEXITCODE=$null; ",
                    "try {{ ",
                    "& ([ScriptBlock]::Create({command})); ",
                    "if ($null -ne $LASTEXITCODE) {{ exit ([int]$LASTEXITCODE) }} ",
                    "elseif ($?) {{ exit 0 }} else {{ exit 1 }} ",
                    "}} catch {{ [Console]::Error.WriteLine(($_ | Out-String)); exit 1 }}"
                ),
                command = command_q,
            );
            let payload = super::windows::powershell_encoded_payload(&script);
            (
                "powershell.exe",
                windows_command_line_args(&["-NoProfile", "-EncodedCommand", payload.as_str()]),
            )
        }
        ShellType::Cmd => ("cmd.exe", windows_cmd_arguments(user_command)),
        ShellType::Posix | ShellType::Fish => unreachable!("not a Windows shell"),
    }
}

/// Build the raw command line consumed by `cmd.exe` itself.
///
/// The command following `/c` is not a normal CRT argument: CMD reparses the
/// remaining command line using its own quote, metacharacter, and expansion
/// rules. Passing `user_command` through `windows_command_line_arg` would turn
/// inner quotes into `\"`, which CMD does not treat as an escaped quote. In
/// particular, an explicit nested `powershell.exe -Command "..."` would then
/// execute the quoted body as literal text instead of as PowerShell code.
fn windows_cmd_arguments(user_command: &str) -> String {
    let command = user_command.trim_start();
    if command.starts_with('"') {
        // CMD strips the first and final quote around `/s /c` commands. Add
        // the conventional extra pair when the executable path itself is
        // quoted, while leaving every user-supplied inner character intact.
        format!("/d /v:off /s /c \"{command}\"")
    } else {
        format!("/d /v:off /s /c {user_command}")
    }
}

fn windows_command_line_args(arguments: &[&str]) -> String {
    arguments
        .iter()
        .map(|argument| windows_command_line_arg(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn windows_command_line_arg(argument: &str) -> String {
    if argument.is_empty()
        || argument
            .chars()
            .any(|character| matches!(character, ' ' | '\t' | '"'))
    {
        let mut quoted = String::from("\"");
        let mut backslashes = 0;
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    quoted.push(character);
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    } else {
        argument.to_string()
    }
}

/// Build the poll command for the given shell type. Every variant emits:
/// `status=...`, optional `exit=...`, `truncated=0|1`, and bounded base64
/// stdout/stderr fields. Base64 keeps arbitrary output from colliding with the
/// framing protocol and lets Rust handle partial UTF-8 boundaries safely.
fn make_poll_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => make_posix_poll_command(marker, shell_type),
        ShellType::PowerShell | ShellType::Cmd => {
            super::windows::powershell_compressed_command(&make_windows_poll_script(marker))
        }
    }
}

fn make_posix_poll_command(marker: &str, shell_type: ShellType) -> String {
    let status = shell_quote(marker, shell_type);
    let out = shell_quote(&format!("{marker}.out"), shell_type);
    let err = shell_quote(&format!("{marker}.err"), shell_type);
    format!(
        concat!(
            "emit_streams() {{ ",
            "stdout_bytes=0; stderr_bytes=0; truncated=0; ",
            "if [ -f {out} ]; then stdout_bytes=$(wc -c <{out} 2>/dev/null || printf 0); fi; ",
            "if [ -f {err} ]; then stderr_bytes=$(wc -c <{err} 2>/dev/null || printf 0); fi; ",
            "if [ \"$stdout_bytes\" -gt {max} ] 2>/dev/null || [ \"$stderr_bytes\" -gt {max} ] 2>/dev/null; then truncated=1; fi; ",
            "printf 'truncated=%s\\nstdout_b64=' \"$truncated\"; ",
            "if [ -f {out} ]; then tail -c {max} {out} 2>/dev/null | base64 | tr -d '\\r\\n'; fi; ",
            "printf '\\nstderr_b64='; ",
            "if [ -f {err} ]; then tail -c {max} {err} 2>/dev/null | base64 | tr -d '\\r\\n'; fi; ",
            "printf '\\n'; ",
            "}}; ",
            "if [ -f {status} ]; then ",
            "exit_status=$(head -c 32 {status} 2>/dev/null); ",
            "if [ \"$exit_status\" = stopped ]; then printf 'status=stopped\\n'; ",
            "else printf 'status=exited\\nexit=%s\\n' \"$exit_status\"; fi; ",
            "emit_streams; ",
            "elif [ -f {out} ] || [ -f {err} ]; then printf 'status=running\\n'; emit_streams; ",
            "else printf 'status=not_found\\n'; fi"
        ),
        status = status,
        out = out,
        err = err,
        max = DEFAULT_MAX_OUTPUT_BYTES,
    )
}

fn make_windows_poll_script(marker: &str) -> String {
    let marker_q = shell_quote(marker, ShellType::PowerShell);
    format!(
        concat!(
            "$marker=[Environment]::ExpandEnvironmentVariables({marker}); ",
            "$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'; ",
            "$ctlOut=$marker+'.ctl.out'; $ctlErr=$marker+'.ctl.err'; $runner=$marker+'.runner.ps1'; ",
            "function Read-MiaominalTail([string]$path,[int]$limit) {{ ",
            "if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {{ ",
            "return [pscustomobject]@{{Bytes=[byte[]]::new(0);Count=0;Text='';Truncated=$false}} ",
            "}}; ",
            "$stream=$null; ",
            "try {{ ",
            "$share=[IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete; ",
            "$stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,$share); ",
            "$length=$stream.Length; $count=[int][Math]::Min([int64]$limit,$length); ",
            "$bytes=[byte[]]::new($count); $total=0; ",
            "if ($count -gt 0) {{ $stream.Seek($length-$count,[IO.SeekOrigin]::Begin) *> $null }}; ",
            "while ($total -lt $count) {{ ",
            "$read=$stream.Read($bytes,$total,$count-$total); if ($read -le 0) {{ break }}; $total+=$read ",
            "}}; ",
            "$text=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$total); ",
            "return [pscustomobject]@{{Bytes=$bytes;Count=$total;Text=$text;Truncated=($length -gt $limit)}} ",
            "}} catch {{ return [pscustomobject]@{{Bytes=[byte[]]::new(0);Count=0;Text='';Truncated=$false}} }} ",
            "finally {{ if ($null -ne $stream) {{ $stream.Dispose() }} }} ",
            "}}; ",
            "function Get-MiaominalProcessState {{ ",
            "if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) {{ return [pscustomobject]@{{Alive=$false;Diagnostic='job pid metadata was missing'}} }}; ",
            "try {{ ",
            "$metadata=(Read-MiaominalTail $pidFile 4096).Text | ConvertFrom-Json; ",
            "$process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop; ",
            "$actualTicks=$process.StartTime.ToUniversalTime().Ticks; $expectedTicks=[int64]$metadata.start_ticks; ",
            "if ($actualTicks -eq $expectedTicks) {{ return [pscustomobject]@{{Alive=$true;Diagnostic=''}} }}; ",
            "return [pscustomobject]@{{Alive=$false;Diagnostic='job pid identity mismatch'}} ",
            "}} catch {{ return [pscustomobject]@{{Alive=$false;Diagnostic=('job process lookup failed: '+($_.Exception.Message))}} }} ",
            "}}; ",
            "$diagnostic=''; $hasOutput=$false; $processState=Get-MiaominalProcessState; ",
            "if (Test-Path -LiteralPath $marker -PathType Leaf) {{ ",
            "$statusResult=Read-MiaominalTail $marker 64; ",
            "for ($i=0; $i -lt 20 -and -not $statusResult.Text; $i++) {{ Start-Sleep -Milliseconds 10; $statusResult=Read-MiaominalTail $marker 64 }}; ",
            "$status=$statusResult.Text.Trim(); ",
            "if ($status -eq 'stopped') {{ Write-Output 'status=stopped' }} ",
            "elseif ($status -match '^-?[0-9]+$') {{ Write-Output 'status=exited'; Write-Output ('exit='+$status) }} ",
            "else {{ Write-Output 'status=exited'; $statusBytes=(New-Object Text.UTF8Encoding($false)).GetBytes($status); $diagnostic=('job status file was invalid: '+[Convert]::ToBase64String($statusBytes)) }}; ",
            "$hasOutput=$true ",
            "}} elseif ($processState.Alive) {{ Write-Output 'status=running'; $hasOutput=$true ",
            "}} elseif ((Test-Path -LiteralPath $out) -or (Test-Path -LiteralPath $err) -or (Test-Path -LiteralPath $pidFile)) {{ ",
            "Write-Output 'status=exited'; $diagnostic='job process disappeared before writing an exit status'; ",
            "if ($processState.Diagnostic) {{ $diagnostic+=': '+$processState.Diagnostic }}; ",
            "$hasOutput=$true ",
            "}} else {{ Write-Output 'status=not_found' }}; ",
            "if ($hasOutput) {{ ",
            "$stdout=Read-MiaominalTail $out {max}; $stderr=Read-MiaominalTail $err {max}; ",
            "$truncated=$stdout.Truncated -or $stderr.Truncated; ",
            "Write-Output ('truncated='+[int]$truncated); ",
            "Write-Output ('stdout_b64='+[Convert]::ToBase64String($stdout.Bytes,0,$stdout.Count)); ",
            "Write-Output ('stderr_b64='+[Convert]::ToBase64String($stderr.Bytes,0,$stderr.Count)); ",
            "if ($diagnostic) {{ ",
            "$diagnosticBytes=(New-Object Text.UTF8Encoding($false)).GetBytes($diagnostic); ",
            "Write-Output ('diagnostic_b64='+[Convert]::ToBase64String($diagnosticBytes)) ",
            "}} ",
            "}}"
        ),
        marker = marker_q,
        max = DEFAULT_MAX_OUTPUT_BYTES,
    )
}

fn make_cleanup_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let paths = [
                marker.to_string(),
                format!("{marker}.out"),
                format!("{marker}.err"),
                format!("{marker}.pid"),
                format!("{marker}.ctl.out"),
                format!("{marker}.ctl.err"),
                format!("{marker}.runner.ps1"),
            ]
            .into_iter()
            .map(|path| shell_quote(&path, shell_type))
            .collect::<Vec<_>>()
            .join(" ");
            format!("rm -f {paths}")
        }
        ShellType::PowerShell | ShellType::Cmd => {
            let marker_q = shell_quote(marker, ShellType::PowerShell);
            let script = format!(
                concat!(
                    "$marker=[Environment]::ExpandEnvironmentVariables({marker}); ",
                    "Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),($marker+'.pid'),($marker+'.ctl.out'),($marker+'.ctl.err'),($marker+'.runner.ps1')) ",
                    "-Force -ErrorAction SilentlyContinue; ",
                    "$root=Split-Path -Parent $marker; $leaf=Split-Path -Leaf $marker; ",
                    "Get-ChildItem -LiteralPath $root -Filter ($leaf+'.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue; ",
                    "Get-ChildItem -LiteralPath $root -Filter ($leaf+'.pid.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue"
                ),
                marker = marker_q,
            );
            super::windows::powershell_compressed_command(&script)
        }
    }
}

fn make_scavenge_command(shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => format!(
            concat!(
                "find /tmp -maxdepth 1 -type f -name 'miaominal-agent-*.status' -mmin +{minutes} -print 2>/dev/null | ",
                "while IFS= read -r marker; do ",
                "name=${{marker##*/}}; id=${{name#miaominal-agent-}}; id=${{id%.status}}; ",
                "case \"$id\" in ????????-????-????-????-????????????) ;; *) continue ;; esac; ",
                "case \"$id\" in *[!0-9a-fA-F-]*) continue ;; esac; ",
                "rm -f \"$marker\" \"$marker.out\" \"$marker.err\" \"$marker.pid\" ",
                "\"$marker.ctl.out\" \"$marker.ctl.err\" \"$marker.runner.ps1\" \"$marker\".tmp-*; ",
                "printf 'cleaned=%s\\n' \"$id\"; done"
            ),
            minutes = STALE_JOB_HOURS * 60,
        ),
        ShellType::PowerShell | ShellType::Cmd => {
            let script = format!(
                concat!(
                    "$root=[IO.Path]::GetTempPath(); $cutoff=[DateTime]::UtcNow.AddHours(-{hours}); ",
                    "$pattern='^miaominal-agent-([0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}})\\.status$'; ",
                    "function Remove-MiaominalArtifacts([string]$marker,[string]$id) {{ ",
                    "Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),($marker+'.pid'),($marker+'.ctl.out'),($marker+'.ctl.err'),($marker+'.runner.ps1')) ",
                    "-Force -ErrorAction SilentlyContinue; ",
                    "Get-ChildItem -LiteralPath $root -Filter ((Split-Path -Leaf $marker)+'.tmp-*') -File -ErrorAction SilentlyContinue ",
                    "| Remove-Item -Force -ErrorAction SilentlyContinue; ",
                    "Get-ChildItem -LiteralPath $root -Filter ((Split-Path -Leaf $marker)+'.pid.tmp-*') -File -ErrorAction SilentlyContinue ",
                    "| Remove-Item -Force -ErrorAction SilentlyContinue; ",
                    "Write-Output ('cleaned='+$id.ToLowerInvariant()) ",
                    "}}; ",
                    "function Test-MiaominalProcess([string]$pidFile) {{ ",
                    "$stream=$null; ",
                    "try {{ ",
                    "$stream=[IO.File]::Open($pidFile,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite); ",
                    "$count=[int][Math]::Min(4096,$stream.Length); $bytes=[byte[]]::new($count); ",
                    "$read=$stream.Read($bytes,0,$count); ",
                    "$metadata=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$read) | ConvertFrom-Json; ",
                    "$process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop; ",
                    "return $process.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.start_ticks) ",
                    "}} catch {{ return $false }} finally {{ if ($null -ne $stream) {{ $stream.Dispose() }} }} ",
                    "}}; ",
                    "Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status' -File -ErrorAction SilentlyContinue ",
                    "| Where-Object {{ $_.LastWriteTimeUtc -lt $cutoff -and $_.Name -match $pattern }} ",
                    "| ForEach-Object {{ Remove-MiaominalArtifacts $_.FullName $Matches[1] }}; ",
                    "Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.pid' -File -ErrorAction SilentlyContinue ",
                    "| Where-Object {{ $_.LastWriteTimeUtc -lt $cutoff }} | ForEach-Object {{ ",
                    "$statusName=$_.Name.Substring(0,$_.Name.Length-4); ",
                    "if ($statusName -match $pattern -and -not (Test-MiaominalProcess $_.FullName)) {{ ",
                    "$marker=Join-Path $root $statusName; Remove-MiaominalArtifacts $marker $Matches[1] ",
                    "}} ",
                    "}}; ",
                    "Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.out' -File -ErrorAction SilentlyContinue ",
                    "| Where-Object {{ $_.LastWriteTimeUtc -lt $cutoff }} | ForEach-Object {{ ",
                    "$statusName=$_.Name.Substring(0,$_.Name.Length-4); $marker=Join-Path $root $statusName; ",
                    "if ($statusName -match $pattern -and -not (Test-Path -LiteralPath $marker) -and -not (Test-Path -LiteralPath ($marker+'.pid'))) {{ ",
                    "Remove-MiaominalArtifacts $marker $Matches[1] ",
                    "}} ",
                    "}}; ",
                    "$runnerSuffix='.runner.ps1'; ",
                    "Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.runner.ps1' -File -ErrorAction SilentlyContinue ",
                    "| Where-Object {{ $_.LastWriteTimeUtc -lt $cutoff }} | ForEach-Object {{ ",
                    "$statusName=$_.Name.Substring(0,$_.Name.Length-$runnerSuffix.Length); $marker=Join-Path $root $statusName; ",
                    "if ($statusName -match $pattern -and -not (Test-MiaominalProcess ($marker+'.pid'))) {{ ",
                    "Remove-MiaominalArtifacts $marker $Matches[1] ",
                    "}} ",
                    "}}; ",
                    "$pidTmpPattern='^miaominal-agent-[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\\.status\\.pid\\.tmp-[0-9a-fA-F]+$'; ",
                    "Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.pid.tmp-*' -File -ErrorAction SilentlyContinue ",
                    "| Where-Object {{ $_.LastWriteTimeUtc -lt $cutoff -and $_.Name -match $pidTmpPattern }} ",
                    "| Remove-Item -Force -ErrorAction SilentlyContinue"
                ),
                hours = STALE_JOB_HOURS,
            );
            super::windows::powershell_compressed_command(&script)
        }
    }
}

async fn scavenge_jobs(channel: &AgentExecChannel, shell_type: ShellType) {
    let command = make_scavenge_command(shell_type);
    if ensure_windows_command_fits(&command, shell_type).is_err() {
        return;
    }
    let Ok(output) = channel.exec(command).await else {
        return;
    };
    for id in output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cleaned="))
    {
        let job_id = AgentJobId(id.to_string());
        if job_id.remote_marker_for_shell(shell_type).is_ok() {
            let _ = channel.jobs().remove(&job_id);
        }
    }
}

async fn detected_job_shell(channel: &AgentExecChannel) -> ShellType {
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }
    channel.shell_type()
}

fn ensure_windows_command_fits(command: &str, shell_type: ShellType) -> AgentResult<()> {
    if shell_type == ShellType::Cmd && command.len() >= WINDOWS_CMD_MAX_COMMAND_BYTES {
        return Err(AgentError::Backend(anyhow::anyhow!(
            "generated Windows background-job command exceeds CMD's 8191-byte limit; put the long command in a script and start that script instead"
        )));
    }
    Ok(())
}

fn make_stop_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => make_posix_stop_command(marker, shell_type),
        ShellType::PowerShell | ShellType::Cmd => {
            super::windows::powershell_compressed_command(&make_windows_stop_script(marker))
        }
    }
}

fn make_posix_stop_command(marker: &str, shell_type: ShellType) -> String {
    let status = shell_quote(marker, shell_type);
    let out = shell_quote(&format!("{marker}.out"), shell_type);
    let err = shell_quote(&format!("{marker}.err"), shell_type);
    let tmp = shell_quote(&format!("{marker}.tmp"), shell_type);
    format!(
        concat!(
            "if [ -f {status} ]; then rm -f {out} {err}; printf 'already_finished\\n'; ",
            "else pkill -f {status} 2>/dev/null || true; rm -f {out} {err}; ",
            "printf 'stopped' >{tmp} && mv -f {tmp} {status}; printf 'stopped\\n'; fi"
        ),
        status = status,
        out = out,
        err = err,
        tmp = tmp,
    )
}

fn make_windows_stop_script(marker: &str) -> String {
    let marker_q = shell_quote(marker, ShellType::PowerShell);
    format!(
        concat!(
            "$marker=[Environment]::ExpandEnvironmentVariables({marker}); ",
            "$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'; ",
            "$ctlOut=$marker+'.ctl.out'; $ctlErr=$marker+'.ctl.err'; $runner=$marker+'.runner.ps1'; ",
            "$artifacts=@($marker,$out,$err,$pidFile,$ctlOut,$ctlErr,$runner); ",
            "if (-not ($artifacts | Where-Object {{ Test-Path -LiteralPath $_ }})) {{ Write-Output 'not_found'; exit 0 }}; ",
            "if (Test-Path -LiteralPath $marker -PathType Leaf) {{ ",
            "Remove-Item -LiteralPath @($out,$err,$pidFile,$ctlOut,$ctlErr,$runner) -Force -ErrorAction SilentlyContinue; ",
            "Write-Output 'already_finished'; exit 0 ",
            "}}; ",
            "$valid=$false; $metadata=$null; ",
            "if (Test-Path -LiteralPath $pidFile -PathType Leaf) {{ ",
            "try {{ ",
            "$stream=[IO.File]::Open($pidFile,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite); ",
            "$count=[int][Math]::Min(4096,$stream.Length); $bytes=[byte[]]::new($count); ",
            "$read=$stream.Read($bytes,0,$count); $stream.Dispose(); ",
            "$metadata=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$read) | ConvertFrom-Json; ",
            "$process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop; ",
            "$valid=$process.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.start_ticks) ",
            "}} catch {{ $valid=$false }} ",
            "}}; ",
            "$childValid=$false; $childProcess=$null; ",
            "if ($valid -and $null -ne $metadata.child_pid -and $null -ne $metadata.child_start_ticks) {{ ",
            "try {{ $childProcess=Get-Process -Id ([int]$metadata.child_pid) -ErrorAction Stop; ",
            "$childValid=$childProcess.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.child_start_ticks) ",
            "}} catch {{ $childValid=$false }} ",
            "}}; ",
            "if ($valid) {{ ",
            "$targetProcessId=[int]$metadata.pid; $taskkillOutput=(& taskkill.exe /T /F /PID $targetProcessId 2>&1 | Out-String); ",
            "$killExitCode=$LASTEXITCODE; $stopped=$process.WaitForExit(1000); ",
            "if (-not $stopped) {{ ",
            "if ($childValid -and -not $childProcess.HasExited) {{ ",
            "try {{ $childProcess.Kill(); $childProcess.WaitForExit(5000) *> $null }} catch {{}} ",
            "}}; ",
            "try {{ if (-not $process.HasExited) {{ $process.Kill() }}; $stopped=$process.WaitForExit(5000) }} catch {{ $stopped=$false }} ",
            "}}; ",
            "if (-not $stopped -or ($childValid -and -not $childProcess.HasExited)) {{ ",
            "throw ('failed to stop job process tree; taskkill exit code '+$killExitCode+': '+$taskkillOutput) ",
            "}} ",
            "}}; ",
            "Remove-Item -LiteralPath @($out,$err,$pidFile,$ctlOut,$ctlErr,$runner) -Force -ErrorAction SilentlyContinue; ",
            "$statusTmp=$marker+'.tmp-'+[Guid]::NewGuid().ToString('N'); ",
            "[IO.File]::WriteAllText($statusTmp,'stopped',(New-Object Text.UTF8Encoding($false))); ",
            "Move-Item -LiteralPath $statusTmp -Destination $marker -Force; ",
            "Write-Output 'stopped'"
        ),
        marker = marker_q,
    )
}

pub async fn start_job(channel: &AgentExecChannel, args: StartJobArgs) -> AgentResult<ToolOutput> {
    let cwd = resolve_workspace_path(args.cwd.as_deref().unwrap_or("."))?;
    // Approval execution and later agent turns can use a newly constructed
    // channel. Revalidate before launching so a stale profile/cache cannot
    // select CMD when this SSH exec channel is actually PowerShell.
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::refresh_exec_shell_detected(channel).await;
    }
    let shell_type = channel.shell_type();
    let job_id = AgentJobId::new();
    let marker = job_id.remote_marker_for_shell(shell_type)?;
    let command = make_start_job_command(shell_type, &cwd, &args.command, &marker);
    let launch = make_start_job_launch(shell_type, &command, &marker);
    ensure_windows_command_fits(&launch, shell_type)?;
    scavenge_jobs(channel, shell_type).await;

    let launch_output = channel.exec(launch).await?;
    let marker = launch_output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches('\'')
        .to_string();
    let expected_name = format!("miaominal-agent-{}.status", job_id.0);
    if marker.is_empty()
        || !marker
            .to_ascii_lowercase()
            .ends_with(&expected_name.to_ascii_lowercase())
    {
        return Err(AgentError::Backend(anyhow::anyhow!(
            "job launcher did not return the expected marker path"
        )));
    }
    let job_id = channel
        .jobs()
        .insert_remote_job_with_id(job_id, args.command, marker);
    Ok(ToolOutput::JobStarted {
        job_id,
        exec_shell: match shell_type {
            ShellType::Posix => "posix-sh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
        .into(),
        poll_after_ms: DEFAULT_POLL_AFTER_MS,
        next_action: "Poll this job with poll_job until status is exited, or use list_jobs if you lose the job_id. Use run_shell instead of start_job for short commands."
            .into(),
    })
}

pub async fn list_jobs(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    scavenge_jobs(channel, shell_type).await;
    Ok(ToolOutput::JobList {
        jobs: channel.jobs().list()?,
    })
}

pub async fn poll_job(channel: &AgentExecChannel, args: PollJobArgs) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    let marker = channel
        .jobs()
        .remote_marker_for_shell(&args.job_id, shell_type)?;
    let poll_command = make_poll_command(&marker, shell_type);
    ensure_windows_command_fits(&poll_command, shell_type)?;
    let output = channel.exec(poll_command).await?;
    let result = parse_poll_output(args.job_id.clone(), &output)?;
    if matches!(result.status, JobStatus::Exited | JobStatus::Stopped) {
        let cleanup_command = make_cleanup_command(&marker, shell_type);
        if ensure_windows_command_fits(&cleanup_command, shell_type).is_ok() {
            let _ = channel.exec(cleanup_command).await;
        }
        let _ = channel.jobs().remove(&args.job_id);
    } else if result.status == JobStatus::NotFound {
        let _ = channel.jobs().remove(&args.job_id);
    }
    Ok(ToolOutput::JobPoll { result })
}

fn parse_poll_output(job_id: AgentJobId, output: &str) -> AgentResult<JobPollResult> {
    let normalized = output.replace("\r\n", "\n").replace('\r', "\n");
    let status = normalized
        .lines()
        .find_map(|line| line.strip_prefix("status="))
        .ok_or_else(|| {
            AgentError::Backend(anyhow::anyhow!("job poll response is missing status"))
        })?;
    let exit_status = normalized
        .lines()
        .find_map(|line| line.strip_prefix("exit="))
        .and_then(|value| value.trim().parse::<i32>().ok());
    let truncated = normalized
        .lines()
        .find_map(|line| line.strip_prefix("truncated="))
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "True"));

    let mut stderr = encoded_output_field(&normalized, "stderr_b64")?
        .or_else(|| heredoc_section(&normalized, "stderr"))
        .unwrap_or_default();
    if let Some(diagnostic) = encoded_output_field(&normalized, "diagnostic_b64")? {
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&diagnostic);
    }

    Ok(JobPollResult {
        job_id,
        status: match status.trim() {
            "running" => JobStatus::Running,
            "exited" => JobStatus::Exited,
            "not_found" => JobStatus::NotFound,
            "stopped" => JobStatus::Stopped,
            other => {
                return Err(AgentError::Backend(anyhow::anyhow!(
                    "unknown job status `{other}`"
                )));
            }
        },
        exit_status,
        stdout: encoded_output_field(&normalized, "stdout_b64")?
            .or_else(|| heredoc_section(&normalized, "stdout"))
            .unwrap_or_default(),
        stderr,
        truncated,
    })
}

fn encoded_output_field(output: &str, name: &str) -> AgentResult<Option<String>> {
    let prefix = format!("{name}=");
    let Some(value) = output.lines().find_map(|line| line.strip_prefix(&prefix)) else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|error| AgentError::Backend(anyhow::anyhow!("invalid {name}: {error}")))?;
    let mut start = 0;
    while start < bytes.len().min(3) && bytes[start] & 0b1100_0000 == 0b1000_0000 {
        start += 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    if text.len() > DEFAULT_MAX_OUTPUT_BYTES {
        text.truncate(text.floor_char_boundary(DEFAULT_MAX_OUTPUT_BYTES));
    }
    Ok(Some(text))
}

fn heredoc_section(output: &str, name: &str) -> Option<String> {
    let start = format!("{name}<<EOF\n");
    let after_start = output.split_once(&start)?.1;
    let section = after_start.split_once("\nEOF")?.0;
    Some(section.to_string())
}

pub async fn stop_job(channel: &AgentExecChannel, args: StopJobArgs) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    let marker = channel
        .jobs()
        .remote_marker_for_shell(&args.job_id, shell_type)?;
    let command = make_stop_command(&marker, shell_type);
    ensure_windows_command_fits(&command, shell_type)?;
    let content = channel.exec(command).await?;
    let _ = channel.jobs().remove(&args.job_id);
    Ok(ToolOutput::Text {
        content,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    static WINDOWS_SCAVENGE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn start_job_uses_detected_powershell_over_configured_cmd() {
        let mut profile = miaominal_core::profile::SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = ShellType::Cmd;
        let channel = AgentExecChannel::for_profile(
            profile,
            Vec::new(),
            miaominal_secrets::SecretStore::new_locked_vault(),
            miaominal_storage::known_hosts_store::KnownHostsStore::with_path(
                std::env::temp_dir().join("agent-job-detected-shell-known-hosts"),
            ),
        );
        assert_eq!(channel.shell_type(), ShellType::Cmd);

        channel.set_detected_shell(ShellType::PowerShell);
        let job_id = AgentJobId::new();
        let effective_shell = detected_job_shell(&channel).await;
        let marker = job_id
            .remote_marker_for_shell(effective_shell)
            .expect("marker should be generated");
        let command = make_start_job_command(effective_shell, ".", "Write-Output 'hello'", &marker);
        let (program, arguments) = windows_child_command(effective_shell, "Write-Output 'hello'");

        assert_eq!(effective_shell, ShellType::PowerShell);
        assert_eq!(program, "powershell.exe");
        assert!(arguments.contains("-EncodedCommand"));
        assert!(command.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn posix_start_job_command_uses_nohup_sh() {
        let cmd = make_start_job_command(
            ShellType::Posix,
            "/home/user/project",
            "echo hello",
            "/tmp/miaominal-agent-test.status",
        );
        assert!(cmd.contains("nohup sh -lc"), "expected nohup: {cmd}");
        assert!(
            cmd.contains(".status'.out"),
            "expected .out redirect: {cmd}"
        );
        assert!(
            cmd.contains(".status'.err"),
            "expected .err redirect: {cmd}"
        );
    }

    #[test]
    fn posix_start_job_launch_backgrounds_and_prints_marker() {
        let launch = make_start_job_launch(
            ShellType::Posix,
            "nohup sh -lc 'echo hi'",
            "/tmp/marker.status",
        );
        assert!(launch.contains('&'), "expected background &: {launch}");
        assert!(launch.contains("printf '%s'"), "expected printf: {launch}");
    }

    #[test]
    fn posix_poll_is_bounded_and_uses_base64_framing() {
        let command = make_poll_command("/tmp/miaominal-agent-job.status", ShellType::Posix);
        assert!(command.contains("status=running"));
        assert!(command.contains("status=not_found"));
        assert!(command.contains(&format!("tail -c {DEFAULT_MAX_OUTPUT_BYTES}")));
        assert!(command.contains("truncated=%s"));
        assert!(command.contains("stdout_b64="));
        assert!(command.contains("base64"));
        assert!(!command.contains("else cat"));
    }

    #[test]
    fn cleanup_and_scavenge_only_target_miaominal_artifacts() {
        let marker = "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000.status";
        let cleanup = make_cleanup_command(marker, ShellType::Posix);
        assert!(cleanup.contains("runner.ps1"));
        assert!(cleanup.contains(marker));

        let posix_scavenge = make_scavenge_command(ShellType::Posix);
        assert!(posix_scavenge.contains("-mmin +1440"));
        assert!(posix_scavenge.contains("*[!0-9a-fA-F-]*"));

        let windows_scavenge = make_scavenge_command(ShellType::PowerShell);
        assert!(windows_scavenge.len() < 8_191);
        assert!(windows_scavenge.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn cmd_rejects_generated_commands_at_the_platform_limit() {
        let oversized = "x".repeat(WINDOWS_CMD_MAX_COMMAND_BYTES);
        assert!(ensure_windows_command_fits(&oversized, ShellType::Cmd).is_err());
        assert!(ensure_windows_command_fits(&oversized, ShellType::PowerShell).is_ok());
    }

    #[test]
    fn windows_start_uses_independent_process_monitor() {
        for shell_type in [ShellType::PowerShell, ShellType::Cmd] {
            let script = make_windows_start_script(
                shell_type,
                r"C:\Users\user\My Project",
                "echo hello",
                r"%TEMP%\miaominal-agent-test.status",
            );
            assert!(script.contains("MiaominalDetachedProcess"));
            assert!(script.contains("runner.ps1"));
            assert!(!script.contains("Start-Job"));
            assert!(!script.contains(r"\tmp\"));
        }
    }

    #[test]
    fn windows_launcher_resolves_relative_cwd_before_starting_monitor() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            r"relative\project",
            "Write-Output (Get-Location).Path",
            r"%TEMP%\miaominal-agent-test.status",
        );

        assert!(script.contains("ExpandEnvironmentVariables('relative\\project')"));
        assert!(script.contains("$cwdPath=Join-Path $env:USERPROFILE $requestedCwd"));
        assert!(script.contains("Get-Item -LiteralPath $cwdPath"));
        assert!(script.contains("$cwdItem.PSIsContainer"));
        assert!(script.contains("$resolvedCwd=$cwdItem.FullName"));
        assert!(script.contains("SetEnvironmentVariable($cwdEnvName,$resolvedCwd,'Process')"));
        assert!(script.contains("GetEnvironmentVariable"));
        assert!(script.contains("MIAOMINAL_AGENT_JOB_CWD"));
        assert!(script.contains("Remove-Item Env:MIAOMINAL_AGENT_JOB_CWD"));
        assert!(script.contains("$psi.WorkingDirectory=$workingDirectory"));
        assert!(!script.contains("$psi.WorkingDirectory='relative\\project'"));
    }

    #[test]
    fn windows_launcher_failure_stops_monitor_and_retries_artifact_cleanup() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'hello'",
            r"%TEMP%\miaominal-agent-test.status",
        );

        assert!(script.contains("$monitorStartTicks"));
        assert!(script.contains("taskkill.exe /T /F /PID $processId"));
        assert!(script.contains("$process.WaitForExit(5000)"));
        assert!(script.contains("$process.Kill()"));
        assert!(script.contains("$childStartTicks=[int64]$process.StartTime"));
        assert!(script.contains("taskkill.exe /T /F /PID $child.Id"));
        assert!(script.contains("$child.WaitForExit(5000)"));
        assert!(script.contains("$i -lt 1000"));
        assert!(script.contains("Remove-MiaominalLaunchArtifacts; Start-Sleep -Milliseconds 100; Remove-MiaominalLaunchArtifacts"));
        assert!(script.contains("$leaf+'.pid.tmp-*'"));
        assert!(script.contains("SetEnvironmentVariable($cwdEnvName,$previousCwdEnv,'Process')"));
        assert!(script.contains("artifacts were preserved for scavenging"));
        let add_type = script.find("Add-Type -TypeDefinition").unwrap();
        let write_runner = script.find("[IO.File]::WriteAllText($runner").unwrap();
        assert!(add_type < write_runner);
    }

    #[test]
    fn windows_monitor_publishes_its_identity_before_starting_child() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'hello'",
            r"%TEMP%\miaominal-agent-test.status",
        );

        let initial_metadata = script
            .find("$monitorMetadata=@{pid=$self.Id")
            .expect("monitor-only metadata");
        let child_start = script.find("[void]$process.Start()").expect("child start");
        let child_metadata = script
            .find("$monitorMetadata[''child_pid'']=$process.Id")
            .expect("child metadata update");
        assert!(initial_metadata < child_start);
        assert!(child_start < child_metadata);
        assert!(script.contains("Publish-MiaominalPidMetadata $monitorMetadata"));
    }

    #[test]
    fn cmd_child_preserves_nested_powershell_command_quotes() {
        let user_command = r#"powershell.exe -NoProfile -Command "Write-Output 'nested value'""#;
        let (program, arguments) = windows_child_command(ShellType::Cmd, user_command);

        assert_eq!(program, "cmd.exe");
        assert_eq!(arguments, format!("/d /v:off /s /c {user_command}"));
        assert!(!arguments.contains(r#"\""#));
    }

    #[test]
    fn cmd_child_adds_outer_quotes_only_for_a_quoted_executable() {
        let user_command = r#""C:\Program Files\PowerShell\powershell.exe" -NoProfile -Command "Write-Output 'quoted path'""#;
        let (_, arguments) = windows_child_command(ShellType::Cmd, user_command);

        assert_eq!(arguments, format!("/d /v:off /s /c \"{user_command}\""));
        assert!(arguments.contains(r#"-Command "Write-Output 'quoted path'""#));
    }

    #[test]
    fn windows_poll_reads_only_bounded_file_tail() {
        let script = make_windows_poll_script(r"%TEMP%\miaominal-agent-test.status");
        assert!(script.contains("Read-MiaominalTail"));
        assert!(script.contains("[IO.File]::Open"));
        assert!(script.contains(&DEFAULT_MAX_OUTPUT_BYTES.to_string()));
        assert!(script.contains("start_ticks"));
        assert!(script.contains("stdout_b64"));
        assert!(!script.contains("ReadAllBytes"));
        assert!(!script.contains("Get-Job"));
        assert!(!script.contains("Receive-Job"));
    }

    #[test]
    fn windows_stop_validates_pid_and_kills_process_tree() {
        let script = make_windows_stop_script(r"%TEMP%\miaominal-agent-test.status");
        assert!(script.contains("start_ticks"));
        assert!(script.contains("taskkill.exe /T /F /PID"));
        assert!(script.contains("WriteAllText"));
        assert!(!script.contains("Stop-Job"));
        let command = make_stop_command(
            r"%TEMP%\miaominal-agent-00000000-0000-0000-0000-000000000000.status",
            ShellType::Cmd,
        );
        assert!(
            command.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
            "stop command was {} bytes",
            command.len()
        );
    }

    #[test]
    fn parses_exited_job_poll_output() {
        let job_id = AgentJobId::new();
        let result = parse_poll_output(
            job_id.clone(),
            "status=exited\nexit=0\ntruncated=0\nstdout<<EOF\nhello\nEOF\nstderr<<EOF\nwarn\nEOF",
        )
        .unwrap();

        assert_eq!(result.job_id, job_id);
        assert_eq!(result.status, JobStatus::Exited);
        assert_eq!(result.exit_status, Some(0));
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "warn");
        assert!(!result.truncated);
    }

    #[test]
    fn parses_crlf_and_truncation_flag() {
        let result = parse_poll_output(
            AgentJobId::new(),
            "status=running\r\ntruncated=1\r\nstdout<<EOF\r\nlatest\r\nEOF\r\nstderr<<EOF\r\n\r\nEOF\r\n",
        )
        .unwrap();

        assert_eq!(result.status, JobStatus::Running);
        assert_eq!(result.stdout, "latest");
        assert!(result.truncated);
    }

    #[test]
    fn base64_framing_preserves_eof_lines_and_diagnostics() {
        let stdout = base64::engine::general_purpose::STANDARD.encode(b"before\nEOF\nafter");
        let stderr = base64::engine::general_purpose::STANDARD.encode(b"warning");
        let diagnostic = base64::engine::general_purpose::STANDARD.encode(b"process disappeared");
        let output = format!(
            "status=exited\ntruncated=0\nstdout_b64={stdout}\nstderr_b64={stderr}\ndiagnostic_b64={diagnostic}\n"
        );
        let result = parse_poll_output(AgentJobId::new(), &output).unwrap();

        assert_eq!(result.stdout, "before\nEOF\nafter");
        assert_eq!(result.stderr, "warning\nprocess disappeared");
    }

    #[test]
    fn base64_tail_drops_partial_utf8_prefix_and_stays_bounded() {
        let mut bytes = vec![0x82, 0xac];
        let valid_prefix = "最新日志🚀".as_bytes();
        bytes.extend_from_slice(valid_prefix);
        bytes.extend(std::iter::repeat_n(
            b'x',
            DEFAULT_MAX_OUTPUT_BYTES - 2 - valid_prefix.len(),
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let output = format!("stdout_b64={encoded}\n");
        let text = encoded_output_field(&output, "stdout_b64")
            .unwrap()
            .unwrap();

        assert!(text.starts_with("最新日志🚀"));
        assert!(text.ends_with('x'));
        assert!(!text.contains('\u{fffd}'));
        assert!(text.len() <= DEFAULT_MAX_OUTPUT_BYTES);
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn parses_missing_job_poll_output() {
        let result = parse_poll_output(AgentJobId::new(), "status=not_found\n").unwrap();

        assert_eq!(result.status, JobStatus::NotFound);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn parses_stopped_job_poll_output() {
        let result = parse_poll_output(
            AgentJobId::new(),
            "status=stopped\ntruncated=0\nstdout<<EOF\npartial\nEOF\nstderr<<EOF\n\nEOF",
        )
        .unwrap();

        assert_eq!(result.status, JobStatus::Stopped);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "partial");
    }

    #[test]
    fn heredoc_extracts_named_section() {
        let output = "stdout<<EOF\nline1\nline2\nEOF\nstderr<<EOF\nerr1\nEOF";
        assert_eq!(heredoc_section(output, "stdout").unwrap(), "line1\nline2");
        assert_eq!(heredoc_section(output, "stderr").unwrap(), "err1");
        assert!(heredoc_section(output, "missing").is_none());
    }

    #[test]
    fn fish_start_job_uses_nohup_like_posix() {
        let cmd = make_start_job_command(
            ShellType::Fish,
            "/home/user/project",
            "echo fish",
            "/tmp/fish.status",
        );
        assert!(cmd.contains("nohup sh -lc"), "Fish also uses nohup: {cmd}");
    }

    #[cfg(windows)]
    fn windows_command_output(command: &str, shell_type: ShellType) -> std::process::Output {
        windows_command_output_with_user_profile(command, shell_type, None)
    }

    #[cfg(windows)]
    fn windows_command_output_with_user_profile(
        command: &str,
        shell_type: ShellType,
        user_profile: Option<&std::path::Path>,
    ) -> std::process::Output {
        let mut process = match shell_type {
            ShellType::Cmd => {
                let mut process = std::process::Command::new("cmd.exe");
                process.args(["/d", "/c", command]);
                process
            }
            ShellType::PowerShell => {
                let payload = command
                    .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
                    .expect("generated PowerShell command prefix");
                let mut process = std::process::Command::new("powershell.exe");
                process.args(["-NoProfile", "-EncodedCommand", payload]);
                process
            }
            ShellType::Posix | ShellType::Fish => unreachable!("Windows integration shell"),
        };
        if let Some(user_profile) = user_profile {
            process.env("USERPROFILE", user_profile);
        }
        process.output().expect("execute generated command")
    }

    #[cfg(windows)]
    fn execute_windows_command(command: &str, shell_type: ShellType) -> String {
        let output = windows_command_output(command, shell_type);
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(windows)]
    fn run_windows_job(shell_type: ShellType, command: &str, expected_exit: i32) -> JobPollResult {
        run_windows_job_in_cwd(shell_type, ".", command, expected_exit)
    }

    #[cfg(windows)]
    fn run_windows_job_in_cwd(
        shell_type: ShellType,
        cwd: &str,
        command: &str,
        expected_exit: i32,
    ) -> JobPollResult {
        run_windows_job_in_cwd_with_user_profile(shell_type, cwd, command, expected_exit, None)
    }

    #[cfg(windows)]
    fn run_windows_job_in_cwd_with_user_profile(
        shell_type: ShellType,
        cwd: &str,
        command: &str,
        expected_exit: i32,
        user_profile: Option<&std::path::Path>,
    ) -> JobPollResult {
        use std::time::{Duration, Instant};

        let job_id = AgentJobId::new();
        let logical_marker = job_id.remote_marker_for_shell(shell_type).unwrap();
        let start = make_start_job_command(shell_type, cwd, command, &logical_marker);
        if shell_type == ShellType::Cmd {
            assert!(
                start.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                "start command was {} bytes",
                start.len()
            );
        }
        let start_output =
            windows_command_output_with_user_profile(&start, shell_type, user_profile);
        assert!(
            start_output.status.success(),
            "job start failed: {}",
            String::from_utf8_lossy(&start_output.stderr)
        );
        let marker = String::from_utf8_lossy(&start_output.stdout)
            .trim()
            .to_string();
        assert!(
            marker.to_ascii_lowercase().contains("\\temp\\"),
            "unexpected launcher output: {marker:?}"
        );
        assert!(
            marker
                .to_ascii_lowercase()
                .starts_with(&std::env::temp_dir().to_string_lossy().to_ascii_lowercase()),
            "marker was outside the Windows temp directory: {marker}"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            let poll_command = make_poll_command(&marker, shell_type);
            if shell_type == ShellType::Cmd {
                assert!(
                    poll_command.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                    "poll command was {} bytes",
                    poll_command.len()
                );
            }
            let poll = execute_windows_command(&poll_command, shell_type);
            let result = parse_poll_output(job_id.clone(), &poll).unwrap();
            if result.status == JobStatus::Exited {
                break result;
            }
            assert_eq!(result.status, JobStatus::Running);
            assert!(Instant::now() < deadline, "job did not exit in time");
            std::thread::sleep(Duration::from_millis(100));
        };

        assert!(std::path::Path::new(&marker).exists());
        let cleanup = make_cleanup_command(&marker, shell_type);
        if shell_type == ShellType::Cmd {
            assert!(
                cleanup.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                "cleanup command was {} bytes",
                cleanup.len()
            );
        }
        execute_windows_command(&cleanup, shell_type);
        assert!(!std::path::Path::new(&marker).exists());
        assert!(!std::path::Path::new(&format!("{marker}.out")).exists());
        assert!(!std::path::Path::new(&format!("{marker}.err")).exists());
        assert_eq!(
            result.exit_status,
            Some(expected_exit),
            "unexpected job result: status={:?}, stdout={:?}, stderr={:?}",
            result.status,
            result.stdout,
            result.stderr
        );
        result
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_survives_separate_powershell_poll_processes() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "Start-Sleep -Milliseconds 500; Write-Output 'hello'; exit 7",
            7,
        );
        assert_eq!(result.stdout.trim(), "hello");
        assert!(!result.truncated);
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_dot_cwd_is_resolved_from_user_profile() {
        let user_profile =
            std::env::temp_dir().join(format!("miaominal-job-home-{}", AgentJobId::new().0));
        std::fs::create_dir_all(&user_profile).unwrap();
        let expected_cwd = user_profile.canonicalize().unwrap();
        let result = run_windows_job_in_cwd_with_user_profile(
            ShellType::PowerShell,
            ".",
            "[Console]::Out.Write([Environment]::CurrentDirectory); exit 0",
            0,
            Some(&user_profile),
        );
        let actual_cwd = std::path::Path::new(result.stdout.trim())
            .canonicalize()
            .unwrap();
        let _ = std::fs::remove_dir_all(&user_profile);

        assert_eq!(actual_cwd, expected_cwd);
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_relative_cwd_is_resolved_from_user_profile() {
        let user_profile =
            std::env::temp_dir().join(format!("miaominal-job-home-{}", AgentJobId::new().0));
        let relative_cwd = format!("miaominal-job-cwd-{}", AgentJobId::new().0);
        let directory = user_profile.join(&relative_cwd);
        std::fs::create_dir_all(&directory).unwrap();
        let expected_cwd = directory.canonicalize().unwrap();
        let result = run_windows_job_in_cwd_with_user_profile(
            ShellType::PowerShell,
            &relative_cwd,
            "[Console]::Out.Write([Environment]::CurrentDirectory); exit 0",
            0,
            Some(&user_profile),
        );
        let actual_cwd = std::path::Path::new(result.stdout.trim())
            .canonicalize()
            .unwrap();
        let _ = std::fs::remove_dir_all(&user_profile);

        assert_eq!(actual_cwd, expected_cwd);
    }

    #[cfg(windows)]
    #[test]
    fn windows_launcher_timeout_cleans_monitor_and_artifacts() {
        use std::path::PathBuf;
        use std::process::Command;

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir()
            .join(format!("miaominal-agent-{}.status", job_id.0))
            .to_string_lossy()
            .into_owned();
        let monitor_pid_probe = format!("{marker}.test-monitor-pid");
        let mut launcher = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'never reached'",
            &marker,
        );

        let monitor_start = "$ErrorActionPreference=''Stop''; $marker=";
        let stalled_monitor = "$ErrorActionPreference=''Stop''; Start-Sleep -Seconds 30; $marker=";
        let stalled_launcher = launcher.replacen(monitor_start, stalled_monitor, 1);
        assert_ne!(
            stalled_launcher, launcher,
            "monitor stall hook was not injected"
        );
        launcher = stalled_launcher;

        let detached_start = "$monitorPid=[MiaominalDetachedProcess]::Start($powershell,$monitorArgs,(Split-Path -Parent $runner)); ";
        let monitor_pid_probe_q = shell_quote(&monitor_pid_probe, ShellType::PowerShell);
        let instrumented_start = format!(
            "{detached_start}[IO.File]::WriteAllText({monitor_pid_probe_q},[string]$monitorPid); "
        );
        let instrumented_launcher = launcher.replacen(detached_start, &instrumented_start, 1);
        assert_ne!(
            instrumented_launcher, launcher,
            "monitor PID probe was not injected"
        );

        let command = super::super::windows::powershell_compressed_command(&instrumented_launcher);
        let output = windows_command_output(&command, ShellType::PowerShell);
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let timed_out = !output.status.success()
            && diagnostic.contains("job monitor failed to publish metadata");
        let monitor_pid: u32 = std::fs::read_to_string(&monitor_pid_probe)
            .expect("launcher should publish the test monitor PID")
            .trim()
            .parse()
            .expect("monitor PID should be numeric");
        let process_probe = format!(
            "if (Get-Process -Id {monitor_pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
        );
        let monitor_alive = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &process_probe])
            .status()
            .expect("probe monitor process")
            .success();
        let artifact_paths = [
            PathBuf::from(&marker),
            PathBuf::from(format!("{marker}.out")),
            PathBuf::from(format!("{marker}.err")),
            PathBuf::from(format!("{marker}.pid")),
            PathBuf::from(format!("{marker}.ctl.out")),
            PathBuf::from(format!("{marker}.ctl.err")),
            PathBuf::from(format!("{marker}.runner.ps1")),
        ];
        let leftovers = artifact_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();

        let _ = Command::new("taskkill.exe")
            .args(["/T", "/F", "/PID", &monitor_pid.to_string()])
            .output();
        for path in artifact_paths {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&monitor_pid_probe);

        assert!(
            timed_out,
            "launcher did not enter the publication timeout path"
        );
        assert!(
            !monitor_alive && leftovers.is_empty(),
            "launcher timeout leaked monitor_alive={monitor_alive}, artifacts={leftovers:?}, diagnostic={diagnostic}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_launcher_create_process_failure_removes_runner() {
        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir()
            .join(format!("miaominal-agent-{}.status", job_id.0))
            .to_string_lossy()
            .into_owned();
        let mut launcher = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'never reached'",
            &marker,
        );
        let real_powershell = "$powershell=Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\powershell.exe'; ";
        let missing_powershell = format!(
            "$powershell=Join-Path $env:TEMP 'miaominal-agent-missing-{}.exe'; ",
            job_id.0
        );
        let replaced = launcher.replacen(real_powershell, &missing_powershell, 1);
        assert_ne!(replaced, launcher, "PowerShell path hook was not injected");
        launcher = replaced;

        let command = super::super::windows::powershell_compressed_command(&launcher);
        let output = windows_command_output(&command, ShellType::PowerShell);
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let artifacts = [
            std::path::PathBuf::from(&marker),
            std::path::PathBuf::from(format!("{marker}.out")),
            std::path::PathBuf::from(format!("{marker}.err")),
            std::path::PathBuf::from(format!("{marker}.pid")),
            std::path::PathBuf::from(format!("{marker}.runner.ps1")),
        ];
        let leftovers = artifacts
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();
        for path in artifacts {
            let _ = std::fs::remove_file(path);
        }

        assert!(!output.status.success());
        assert!(diagnostic.contains("CreateProcess failed"), "{diagnostic}");
        assert!(
            leftovers.is_empty(),
            "CreateProcess failure leaked artifacts: {leftovers:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_uses_temp_artifacts_and_survives_separate_polls() {
        let result = run_windows_job(
            ShellType::Cmd,
            "ping -n 2 127.0.0.1 >nul & echo cmd-output & exit /b 9",
            9,
        );
        assert_eq!(result.stdout.trim(), "cmd-output");
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_preserves_nested_powershell_command_quotes_and_exit_status() {
        let result = run_windows_job(
            ShellType::Cmd,
            "powershell.exe -NoProfile -Command \"Start-Sleep -Milliseconds 1200; Write-Output 'JOB_STDOUT_OK'; [Console]::Error.WriteLine('JOB_STDERR_OK'); exit 7\"",
            7,
        );

        assert_eq!(result.stdout.trim(), "JOB_STDOUT_OK");
        assert_eq!(result.stderr.trim(), "JOB_STDERR_OK");
    }

    #[cfg(windows)]
    #[test]
    fn powershell_job_preserves_explicit_nested_powershell_exit_status() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "powershell.exe -NoProfile -Command \"Start-Sleep -Milliseconds 1200; Write-Output 'JOB_STDOUT_OK'; [Console]::Error.WriteLine('JOB_STDERR_OK'); exit 7\"",
            7,
        );

        assert_eq!(result.stdout.trim(), "JOB_STDOUT_OK");
        assert!(result.stderr.contains("JOB_STDERR_OK"));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_supports_a_quoted_executable_path() {
        let result = run_windows_job(
            ShellType::Cmd,
            r#""%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -Command "Write-Output 'QUOTED_EXE_OK'; exit 5""#,
            5,
        );

        assert_eq!(result.stdout.trim(), "QUOTED_EXE_OK");
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_poll_caps_both_output_streams() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "[Console]::Out.Write(('A'*70000)); [Console]::Error.Write(('B'*70000)); exit 0",
            0,
        );

        assert!(result.truncated);
        assert_eq!(result.stdout.len(), DEFAULT_MAX_OUTPUT_BYTES);
        assert_eq!(result.stderr.len(), DEFAULT_MAX_OUTPUT_BYTES);
        assert!(result.stdout.bytes().all(|byte| byte == b'A'));
        assert!(result.stderr.bytes().all(|byte| byte == b'B'));
    }

    #[cfg(windows)]
    #[test]
    fn windows_stop_kills_job_tree_and_preserves_stopped_status() {
        use std::time::{Duration, Instant};

        let job_id = AgentJobId::new();
        let logical_marker = job_id
            .remote_marker_for_shell(ShellType::PowerShell)
            .unwrap();
        let start = make_start_job_command(
            ShellType::PowerShell,
            ".",
            "Start-Sleep -Seconds 30",
            &logical_marker,
        );
        let started_at = Instant::now();
        let marker = execute_windows_command(&start, ShellType::PowerShell);
        assert!(
            started_at.elapsed() < Duration::from_secs(5),
            "background launcher waited for the 30-second job"
        );
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{marker}.pid")).unwrap())
                .unwrap();
        let monitor_pid = metadata["pid"].as_i64().unwrap();
        let stopped = execute_windows_command(
            &make_stop_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        assert_eq!(stopped.trim(), "stopped");
        let process_gone = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {monitor_pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
                ),
            ])
            .status()
            .unwrap();
        assert!(process_gone.success(), "monitor process survived stop_job");

        let poll = execute_windows_command(
            &make_poll_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        let result = parse_poll_output(job_id, &poll).unwrap();
        assert_eq!(result.status, JobStatus::Stopped);
        assert_eq!(result.exit_status, None);
        execute_windows_command(
            &make_cleanup_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        std::thread::sleep(Duration::from_millis(250));
        assert!(!std::path::Path::new(&marker).exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_scavenger_removes_old_terminal_artifacts() {
        use std::fs;
        use std::process::Command;

        let _guard = WINDOWS_SCAVENGE_TEST_LOCK.lock().unwrap();

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir().join(format!("miaominal-agent-{}.status", job_id.0));
        let out = format!("{}.out", marker.display());
        fs::write(&marker, b"0").unwrap();
        fs::write(&out, b"old output").unwrap();
        let age_script = format!(
            "(Get-Item -LiteralPath '{}').LastWriteTimeUtc=[DateTime]::UtcNow.AddHours(-25)",
            marker.display().to_string().replace('\'', "''")
        );
        let aged = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &age_script])
            .status()
            .unwrap();
        assert!(aged.success());

        let output = execute_windows_command(
            &make_scavenge_command(ShellType::PowerShell),
            ShellType::PowerShell,
        );
        assert!(output.contains(&format!("cleaned={}", job_id.0)));
        assert!(!marker.exists());
        assert!(!std::path::Path::new(&out).exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_scavenger_removes_old_runner_only_and_pid_temp_artifacts() {
        use std::fs;
        use std::process::Command;

        let _guard = WINDOWS_SCAVENGE_TEST_LOCK.lock().unwrap();

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir().join(format!("miaominal-agent-{}.status", job_id.0));
        let runner = format!("{}.runner.ps1", marker.display());
        let pid_tmp = format!("{}.pid.tmp-deadbeef", marker.display());
        fs::write(&runner, b"stale runner").unwrap();
        fs::write(&pid_tmp, b"stale metadata").unwrap();
        let runner_q = runner.replace('\'', "''");
        let pid_tmp_q = pid_tmp.replace('\'', "''");
        let age_script = format!(
            "Get-Item -LiteralPath @('{runner_q}','{pid_tmp_q}') | ForEach-Object {{ $_.LastWriteTimeUtc=[DateTime]::UtcNow.AddHours(-25) }}"
        );
        let aged = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &age_script])
            .status()
            .unwrap();
        assert!(aged.success());

        let output = execute_windows_command(
            &make_scavenge_command(ShellType::PowerShell),
            ShellType::PowerShell,
        );

        assert!(output.contains(&format!("cleaned={}", job_id.0)));
        assert!(!std::path::Path::new(&runner).exists());
        assert!(!std::path::Path::new(&pid_tmp).exists());
    }
}
