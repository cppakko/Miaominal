$root=[IO.Path]::GetTempPath(); $cutoff=[DateTime]::UtcNow.AddHours(-@@HOURS@@)
$pattern='^miaominal-agent-([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\.status$'
function Remove-MiaominalArtifacts([string]$marker,[string]$id) {
    Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),($marker+'.pid'),($marker+'.ctl.out'),($marker+'.ctl.err'),($marker+'.runner.ps1')) -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $root -Filter ((Split-Path -Leaf $marker)+'.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath $root -Filter ((Split-Path -Leaf $marker)+'.pid.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    Write-Output ('cleaned='+$id.ToLowerInvariant())
}
function Test-MiaominalProcess([string]$pidFile) {
    $stream=$null
    try {
        $stream=[IO.File]::Open($pidFile,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
        $count=[int][Math]::Min(4096,$stream.Length); $bytes=[byte[]]::new($count)
        $read=$stream.Read($bytes,0,$count)
        $metadata=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$read) | ConvertFrom-Json
        $process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop
        return $process.StartTime.ToUniversalTime().Ticks -eq ([int64]$metadata.start_ticks)
    } catch { return $false } finally { if ($null -ne $stream) { $stream.Dispose() } }
}
Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff -and $_.Name -match $pattern } |
    ForEach-Object { Remove-MiaominalArtifacts $_.FullName $Matches[1] }
Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.pid' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff } | ForEach-Object {
        $statusName=$_.Name.Substring(0,$_.Name.Length-4)
        if ($statusName -match $pattern -and -not (Test-MiaominalProcess $_.FullName)) {
            $marker=Join-Path $root $statusName; Remove-MiaominalArtifacts $marker $Matches[1]
        }
    }
Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.out' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff } | ForEach-Object {
        $statusName=$_.Name.Substring(0,$_.Name.Length-4); $marker=Join-Path $root $statusName
        if ($statusName -match $pattern -and -not (Test-Path -LiteralPath $marker) -and -not (Test-Path -LiteralPath ($marker+'.pid'))) {
            Remove-MiaominalArtifacts $marker $Matches[1]
        }
    }
$runnerSuffix='.runner.ps1'
Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.runner.ps1' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff } | ForEach-Object {
        $statusName=$_.Name.Substring(0,$_.Name.Length-$runnerSuffix.Length); $marker=Join-Path $root $statusName
        if ($statusName -match $pattern -and -not (Test-MiaominalProcess ($marker+'.pid'))) { Remove-MiaominalArtifacts $marker $Matches[1] }
    }
$pidTmpPattern='^miaominal-agent-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\.status\.pid\.tmp-[0-9a-fA-F]+$'
Get-ChildItem -LiteralPath $root -Filter 'miaominal-agent-*.status.pid.tmp-*' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTimeUtc -lt $cutoff -and $_.Name -match $pidTmpPattern } |
    Remove-Item -Force -ErrorAction SilentlyContinue
