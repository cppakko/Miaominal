$ErrorActionPreference='Stop'
$global:LASTEXITCODE=$null
try {
    & ([ScriptBlock]::Create(@@COMMAND@@))
    if ($null -ne $LASTEXITCODE) { exit ([int]$LASTEXITCODE) }
    elseif ($?) { exit 0 } else { exit 1 }
} catch {
    [Console]::Error.WriteLine(($_ | Out-String))
    exit 1
}
