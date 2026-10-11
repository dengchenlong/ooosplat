# Requires PowerShell 5.1+ (Windows PowerShell or PowerShell 7+).
# Verifies the SAM2 sidecar engine: manifest sanity + weight presence,
# size and SHA-256 as pinned in engines/sam2/manifest.json.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts/verify-sam2.ps1
#   powershell ... -File scripts/verify-sam2.ps1 -InstallRoot D:\ooosplat-engines\sam2
#   powershell ... -File scripts/verify-sam2.ps1 -SkipRuntime  (skip torch/sam2 import probe)

[CmdletBinding()]
param(
    [string]$InstallRoot = "D:\ooosplat-engines\sam2",
    [switch]$SkipRuntime
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$workspace = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $workspace 'engines\sam2\manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Missing SAM2 manifest: $manifestPath"
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$engine = $manifest.engine
if ($engine.name -ne 'SAM2') { throw "Unexpected engine name: $($engine.name)" }
if ($engine.license -ne 'Apache-2.0') { throw 'SAM2 license must be Apache-2.0.' }

$licenseFile = Join-Path $workspace ($engine.licenseFiles[0] -replace '/', [IO.Path]::DirectorySeparatorChar)
if (-not (Test-Path -LiteralPath $licenseFile -PathType Leaf)) {
    throw "Missing SAM2 license file: $($engine.licenseFiles[0])"
}

function Get-Sha256([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '')
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

foreach ($weight in $engine.weights) {
    if ($weight.sha256 -notmatch '^[a-fA-F0-9]{64}$') {
        throw "Weight $($weight.fileName) has no pinned sha256 (found '$($weight.sha256)'). Download it, hash it, and pin the value in engines/sam2/manifest.json."
    }
    $weightPath = Join-Path (Join-Path $InstallRoot 'checkpoints') $weight.fileName
    if (-not (Test-Path -LiteralPath $weightPath -PathType Leaf)) {
        throw "Missing SAM2 weights: $weightPath. Download $($weight.url) first (see engines/sam2/README.md)."
    }
    $size = (Get-Item -LiteralPath $weightPath).Length
    if ($weight.sizeBytes -and $size -ne $weight.sizeBytes) {
        throw "Size mismatch for $($weight.fileName): expected $($weight.sizeBytes) bytes, found $size."
    }
    $actual = Get-Sha256 $weightPath
    if ($actual -ne $weight.sha256) {
        throw "Hash mismatch for $($weight.fileName): expected $($weight.sha256), found $actual."
    }
    Write-Host "Verified $($weight.fileName) ($size bytes, sha256 OK)."
}

if (-not $SkipRuntime) {
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $probe = & python -c "import sam2, torch; print(torch.__version__)" 2>$null
    $probeExit = $LASTEXITCODE
    $ErrorActionPreference = $savedPreference
    if ($probeExit -ne 0) {
        Write-Warning "SAM2 runtime (torch + sam2 package) not importable from the current python; sidecar --backend sam2 will not start. Protocol tests (synthetic backend) remain valid."
    } else {
        Write-Host "SAM2 runtime importable (torch $probe)."
    }
}

Write-Host 'SAM2 engine verification passed.'
