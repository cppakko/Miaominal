param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"

function Resolve-AbsolutePath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }

    return [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Path))
}

function Find-Iscc {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }

    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return $candidate
        }
    }

    throw "Inno Setup Compiler (ISCC.exe) was not found. Install Inno Setup 6 and ensure ISCC.exe is on PATH."
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-AbsolutePath (Join-Path $scriptRoot "..\..")
$issPath = Join-Path $scriptRoot "Miaominal.iss"
$binaryPath = Resolve-AbsolutePath $BinaryPath
$outputPath = Resolve-AbsolutePath $OutputPath
$outputDirectory = Split-Path -Parent $outputPath
$outputExtension = [System.IO.Path]::GetExtension($outputPath)
$outputBaseFilename = [System.IO.Path]::GetFileNameWithoutExtension($outputPath)
$iconPath = Join-Path $repoRoot "assets\generated\app-icon.ico"
$licenseRtfPath = Join-Path $scriptRoot "license.rtf"

if ($outputExtension -ne ".exe") {
    throw "OutputPath must end with .exe for an Inno Setup installer: $outputPath"
}

$isccPath = Find-Iscc

if (-not (Test-Path $issPath)) {
    throw "Expected Inno Setup script at $issPath"
}

if (-not (Test-Path $binaryPath)) {
    throw "Binary not found: $binaryPath"
}

if (-not (Test-Path $iconPath)) {
    throw "Expected generated icon at $iconPath. Build the project once so build.rs can generate it."
}

if (-not (Test-Path $licenseRtfPath)) {
    throw "Expected installer license at $licenseRtfPath"
}

New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null

if (Test-Path $outputPath) {
    Remove-Item -LiteralPath $outputPath -Force
}

$innoArguments = @(
    "/Qp"
    "/O$outputDirectory"
    "/F$outputBaseFilename"
    "/DAppVersion=$Version"
    "/DBinarySource=$binaryPath"
    "/DProductIcon=$iconPath"
    "/DLicenseFile=$licenseRtfPath"
    "/DOutputDir=$outputDirectory"
    "/DOutputBaseFilename=$outputBaseFilename"
    $issPath
)

& $isccPath @innoArguments

if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup failed to build the installer."
}

if (-not (Test-Path $outputPath)) {
    throw "Inno Setup reported success but did not produce an installer at $outputPath"
}
