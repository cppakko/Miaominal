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

function Get-MsiVersion {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RawVersion
    )

    if ($RawVersion -match '^(\d+)\.(\d+)\.(\d+)') {
        return "$($Matches[1]).$($Matches[2]).$($Matches[3])"
    }

    throw "Version '$RawVersion' is not compatible with MSI. Use a semantic version with at least major.minor.patch."
}

$toolPath = Join-Path $HOME ".dotnet\tools"
if (-not ($env:PATH -split ';' | Where-Object { $_ -eq $toolPath })) {
    $env:PATH = "$toolPath;$env:PATH"
}

if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw "The .NET SDK is required to install and run the WiX toolset."
}

$wixVersion = "7.0.0"
$wixEulaId = "wix7"
$wixUiExtension = "WixToolset.UI.wixext/$wixVersion"

if (Get-Command wix -ErrorAction SilentlyContinue) {
    dotnet tool update --global wix --version $wixVersion | Out-Null
} else {
    dotnet tool install --global wix --version $wixVersion | Out-Null
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-AbsolutePath (Join-Path $scriptRoot "..\..")
$wxsPath = Join-Path $scriptRoot "Miaominal.wxs"
$binaryPath = Resolve-AbsolutePath $BinaryPath
$outputPath = Resolve-AbsolutePath $OutputPath
$outputDirectory = Split-Path -Parent $outputPath
$iconPath = Join-Path $repoRoot "assets\generated\app-icon.ico"
$licenseRtfPath = Join-Path $scriptRoot "license.rtf"
$msiVersion = Get-MsiVersion $Version

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

$preExistingCabinets = @{}
Get-ChildItem -Path $outputDirectory -Filter "cab*.cab" -File -ErrorAction SilentlyContinue | ForEach-Object {
    $preExistingCabinets[$_.FullName] = "$($_.Length):$($_.LastWriteTimeUtc.Ticks)"
}

& wix extension add -g -acceptEula $wixEulaId $wixUiExtension | Out-Null

if ($LASTEXITCODE -ne 0) {
    throw "WiX failed to install the UI extension."
}

$wixArguments = @(
    "build"
    "-acceptEula"
    $wixEulaId
    $wxsPath
    "-ext"
    "WixToolset.UI.wixext"
    "-bv"
    "WixUILicenseRtf=$licenseRtfPath"
    "-arch"
    "x64"
    "-d"
    "BinarySource=$binaryPath"
    "-d"
    "ProductIcon=$iconPath"
    "-d"
    "ProductVersion=$msiVersion"
    "-o"
    $outputPath
    "-pdbtype"
    "none"
)

& wix @wixArguments

if ($LASTEXITCODE -ne 0) {
    throw "WiX failed to build the MSI package."
}

if (-not (Test-Path $outputPath)) {
    throw "WiX reported success but did not produce an MSI at $outputPath"
}

$externalCabinets = Get-ChildItem -Path $outputDirectory -Filter "cab*.cab" -File -ErrorAction SilentlyContinue | Where-Object {
    $signature = "$($_.Length):$($_.LastWriteTimeUtc.Ticks)"
    -not $preExistingCabinets.ContainsKey($_.FullName) -or $preExistingCabinets[$_.FullName] -ne $signature
}

if ($externalCabinets) {
    $cabinetNames = ($externalCabinets | Select-Object -ExpandProperty Name) -join ", "
    throw "WiX produced external cabinet files ($cabinetNames). The MSI must be self-contained."
}