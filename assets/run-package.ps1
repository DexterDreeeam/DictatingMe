<#
.SYNOPSIS
    Builds and silently installs the DictatingMe NSIS installer.

.DESCRIPTION
    Validates the local build environment and bundled preset assets, then runs the
    Tauri build. -DebugBuild creates an installable debug build and writes DEBUG-level logs
    under LocalAppData. After a successful build, it stops only the installed Program
    Files instance, performs an NSIS /S overwrite, verifies the installed executable,
    and launches the new version. Downloaded models remain outside the installer.
#>

[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$BuildOnly,
    [string]$ExportDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$AssetsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $AssetsDir
$PresetDir = Join-Path $AssetsDir 'preset\sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01'
$CatalogPath = Join-Path $AssetsDir 'sha.json'
$ManifestPath = Join-Path $AssetsDir 'manifest-cn.json'
$BuildProfile = if ($DebugBuild) { 'debug' } else { 'release' }
$InstallerDir = Join-Path $ProjectDir "target\$BuildProfile\bundle\nsis"
$BuiltExe = Join-Path $ProjectDir "target\$BuildProfile\dictatingme.exe"
$InstallDir = Join-Path $env:ProgramFiles 'DictatingMe'
$InstalledExe = Join-Path $InstallDir 'dictatingme.exe'
$InstalledManifest = Join-Path $InstallDir 'manifest-cn.json'
$AppDataManifest = Join-Path $env:LOCALAPPDATA 'DictatingMe\manifest-cn.json'
$DebugLogDir = Join-Path $env:LOCALAPPDATA 'DictatingMe\logs\debug'

function Assert-Command {
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [Parameter(Mandatory)]
        [string]$Help
    )

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name was not found. $Help"
    }
}

function Assert-File {
    param(
        [Parameter(Mandatory)]
        [string]$Path,
        [Parameter(Mandatory)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Description is missing: $Path"
    }

}

function Get-Sha256 {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $stream = [System.IO.File]::OpenRead($Path)
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $hasher.ComputeHash($stream)
        return [System.BitConverter]::ToString($bytes).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
        $stream.Dispose()
    }
}

function Get-TauriExecutablePayloadSha256 {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $marker = '__TAURI_BUNDLE_TYPE_VAR_'
    $content = [System.Text.Encoding]::ASCII.GetString($bytes)
    $markerOffset = $content.IndexOf($marker, [System.StringComparison]::Ordinal)
    if ($markerOffset -lt 0) {
        throw "Tauri bundle marker is missing: $Path"
    }

    while ($markerOffset -ge 0) {
        $bundleTypeOffset = $markerOffset + $marker.Length
        if ($bundleTypeOffset + 3 -gt $bytes.Length) {
            throw "Tauri bundle marker is truncated: $Path"
        }
        [System.Array]::Clear($bytes, $bundleTypeOffset, 3)
        $markerOffset = $content.IndexOf(
            $marker,
            $markerOffset + $marker.Length,
            [System.StringComparison]::Ordinal
        )
    }

    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $hasher.ComputeHash($bytes)
        return [System.BitConverter]::ToString($hash).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function Get-InstalledProcesses {
    param(
        [Parameter(Mandatory)]
        [string]$ExecutablePath
    )

    $normalizedPath = [System.IO.Path]::GetFullPath($ExecutablePath)
    @(
        Get-CimInstance -ClassName Win32_Process -Filter "Name = 'dictatingme.exe'" |
            Where-Object {
                -not [string]::IsNullOrWhiteSpace($_.ExecutablePath) -and
                [string]::Equals(
                    [System.IO.Path]::GetFullPath($_.ExecutablePath),
                    $normalizedPath,
                    [System.StringComparison]::OrdinalIgnoreCase
                )
            }
    )
}

function Test-InstalledProcessId {
    param(
        [Parameter(Mandatory)]
        [int]$ProcessId,
        [Parameter(Mandatory)]
        [string]$ExecutablePath
    )

    $process = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $ProcessId" `
        -ErrorAction SilentlyContinue
    if ($null -eq $process -or [string]::IsNullOrWhiteSpace($process.ExecutablePath)) {
        return $false
    }
    [string]::Equals(
        [System.IO.Path]::GetFullPath($process.ExecutablePath),
        [System.IO.Path]::GetFullPath($ExecutablePath),
        [System.StringComparison]::OrdinalIgnoreCase
    )
}

function Wait-InstalledProcessExit {
    param(
        [Parameter(Mandatory)]
        [int]$ProcessId,
        [Parameter(Mandatory)]
        [string]$ExecutablePath,
        [Parameter(Mandatory)]
        [int]$TimeoutSeconds
    )

    $deadline = [System.DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([System.DateTime]::UtcNow -lt $deadline) {
        if (-not (Test-InstalledProcessId -ProcessId $ProcessId -ExecutablePath $ExecutablePath)) {
            return $true
        }
        Start-Sleep -Milliseconds 250
    }
    -not (Test-InstalledProcessId -ProcessId $ProcessId -ExecutablePath $ExecutablePath)
}

function Stop-InstalledInstance {
    param(
        [Parameter(Mandatory)]
        [string]$ExecutablePath
    )

    $processes = @(Get-InstalledProcesses -ExecutablePath $ExecutablePath)
    foreach ($process in $processes) {
        $processId = [int]$process.ProcessId
        Write-Host "[DictatingMe] Stopping installed instance PID $processId..." -ForegroundColor Cyan
        Stop-Process -Id $processId -ErrorAction Stop
        if (-not (Wait-InstalledProcessExit `
            -ProcessId $processId `
            -ExecutablePath $ExecutablePath `
            -TimeoutSeconds 30)) {
            Write-Host "[DictatingMe] PID $processId is still exiting; forcing that PID..." `
                -ForegroundColor Yellow
            if (Test-InstalledProcessId -ProcessId $processId -ExecutablePath $ExecutablePath) {
                Stop-Process -Id $processId -Force -ErrorAction Stop
            }
        }
        if (-not (Wait-InstalledProcessExit `
            -ProcessId $processId `
            -ExecutablePath $ExecutablePath `
            -TimeoutSeconds 15)) {
            throw "Installed DictatingMe process $processId did not stop."
        }
    }
}

Write-Host "=== DictatingMe $BuildProfile NSIS build + silent overwrite ===" -ForegroundColor Magenta
Write-Host "Project -> $ProjectDir"
Write-Host "Install -> $InstallDir"
if ($DebugBuild) {
    Write-Host "Debug logs -> $DebugLogDir"
}
Write-Host ''

Assert-Command -Name 'npm.cmd' -Help 'Install Node.js first.'
Assert-Command -Name 'cargo.exe' -Help 'Install the Rust toolchain first.'

if (-not (Test-Path -LiteralPath (Join-Path $ProjectDir 'node_modules') -PathType Container)) {
    throw 'Node dependencies are missing. Run npm.cmd install first.'
}

Assert-File -Path $CatalogPath -Description 'Asset SHA catalog'
Assert-File -Path $ManifestPath -Description 'Chinese asset manifest'
$catalog = Get-Content -LiteralPath $CatalogPath -Raw | ConvertFrom-Json
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or $manifest.locale -ne 'zh-CN') {
    throw 'manifest-cn.json must use schemaVersion 1 and locale zh-CN.'
}
$manifestEntries = @($manifest.speakerRecognition.assets) +
    @($manifest.classifierRecognition.assets) +
    @($manifest.speechModels.models)
$downloadableAssets = @($catalog.assets | Where-Object { -not $_.bundled })
foreach ($asset in $downloadableAssets) {
    $matches = @($manifestEntries | Where-Object { $_.id -eq $asset.id })
    if ($matches.Count -ne 1) {
        throw "manifest-cn.json must contain exactly one entry for $($asset.id)."
    }
    if ([string]::IsNullOrWhiteSpace($matches[0].name) -or @($matches[0].sources).Count -eq 0) {
        throw "manifest-cn.json entry $($asset.id) must contain name and sources."
    }
}
$preset = @($catalog.assets | Where-Object { $_.id -eq 'evoke.sherpa-zipformer-wenetspeech' })
if ($preset.Count -ne 1) {
    throw 'Asset SHA catalog must contain exactly one evoke.sherpa-zipformer-wenetspeech entry.'
}
foreach ($expected in $preset[0].files) {
    $path = Join-Path $PresetDir $expected.path
    Assert-File -Path $path -Description "Preset asset $($expected.path)"
    $item = Get-Item -LiteralPath $path
    if ($item.Length -ne [long]$expected.sizeBytes) {
        throw "Preset asset size mismatch: $path"
    }
    $sha256 = Get-Sha256 -Path $path
    if ($sha256 -ne [string]$expected.sha256) {
        throw "Preset asset SHA-256 mismatch: $path"
    }
}
Write-Host '[DictatingMe] Bundled preset SHA-256 verification passed.' -ForegroundColor Green

Push-Location $ProjectDir
try {
    Write-Host "[DictatingMe] Building $BuildProfile NSIS installer..." -ForegroundColor Cyan
    $buildArguments = @('run', 'tauri', '--', 'build')
    if ($DebugBuild) {
        $buildArguments += '--debug'
    }
    $buildArguments += @('--bundles', 'nsis')
    & npm.cmd @buildArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri package build exited with code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

$installers = @(
    Get-ChildItem -LiteralPath $InstallerDir -Filter '*.exe' -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
)
if ($installers.Count -eq 0) {
    throw "The build completed but no NSIS installer was found in $InstallerDir"
}
Assert-File -Path $BuiltExe -Description 'Built DictatingMe executable'
$builtExecutableHash = Get-TauriExecutablePayloadSha256 -Path $BuiltExe

Write-Host ''
Write-Host '[DictatingMe] Package build completed successfully.' -ForegroundColor Green
Write-Host "Installer -> $($installers[0].FullName)" -ForegroundColor Green
$installerHash = Get-Sha256 -Path $installers[0].FullName
Write-Host "SHA-256 -> $installerHash" -ForegroundColor Green

if (-not [string]::IsNullOrWhiteSpace($ExportDirectory)) {
    $exportPath = if ([System.IO.Path]::IsPathRooted($ExportDirectory)) {
        [System.IO.Path]::GetFullPath($ExportDirectory)
    }
    else {
        [System.IO.Path]::GetFullPath((Join-Path $ProjectDir $ExportDirectory))
    }
    if (Test-Path -LiteralPath $exportPath) {
        Remove-Item -LiteralPath $exportPath -Recurse -Force
    }
    New-Item -ItemType Directory -Path $exportPath -Force | Out-Null
    $exportedInstaller = Join-Path $exportPath $installers[0].Name
    Copy-Item -LiteralPath $installers[0].FullName -Destination $exportedInstaller
    if ((Get-Sha256 -Path $exportedInstaller) -ne $installerHash) {
        throw "Exported installer SHA-256 mismatch: $exportedInstaller"
    }
    Write-Host "[DictatingMe] Exported installer -> $exportedInstaller" -ForegroundColor Green
}

if ($BuildOnly) {
    Write-Host '[DictatingMe] Build-only mode completed; installation was skipped.' -ForegroundColor Green
    return
}

Stop-InstalledInstance -ExecutablePath $InstalledExe

Write-Host '[DictatingMe] Silently overwriting the installed application...' -ForegroundColor Cyan
if ($DebugBuild) {
    New-Item -ItemType Directory -Path $DebugLogDir -Force | Out-Null
    $env:DICTATINGME_DEV_LOG_DIR = $DebugLogDir
}

$installStartedAt = [System.DateTime]::UtcNow
$installerProcess = Start-Process `
    -FilePath $installers[0].FullName `
    -ArgumentList @('/S', '/UPDATE') `
    -PassThru `
    -Wait `
    -WindowStyle Hidden
if ($installerProcess.ExitCode -ne 0) {
    throw "The installer exited with code $($installerProcess.ExitCode)."
}

Assert-File -Path $InstalledExe -Description 'Installed DictatingMe executable'
Assert-File -Path $InstalledManifest -Description 'Installed plaintext manifest'
$installedExecutableHash = Get-TauriExecutablePayloadSha256 -Path $InstalledExe
if ($installedExecutableHash -ne $builtExecutableHash) {
    throw "Installed executable does not match the packaged build: $InstalledExe"
}

Write-Host '[DictatingMe] Silent overwrite completed successfully.' -ForegroundColor Green
$started = @(Get-InstalledProcesses -ExecutablePath $InstalledExe)
if ($started.Count -eq 0) {
    Write-Host '[DictatingMe] Starting the installed application...' -ForegroundColor Cyan
    Start-Process -FilePath $InstalledExe -WorkingDirectory $InstallDir
}

Start-Sleep -Seconds 2
$started = @(Get-InstalledProcesses -ExecutablePath $InstalledExe)
if ($started.Count -eq 0) {
    throw 'The installed DictatingMe application did not remain running.'
}
Write-Host "[DictatingMe] Installed application is running (PID $($started[0].ProcessId))." -ForegroundColor Green

for ($attempt = 0; $attempt -lt 20 -and -not (Test-Path -LiteralPath $AppDataManifest -PathType Leaf); $attempt++) {
    Start-Sleep -Milliseconds 250
}
Assert-File -Path $AppDataManifest -Description 'AppData plaintext manifest'
$sourceManifestHash = Get-Sha256 -Path $ManifestPath
if ((Get-Sha256 -Path $InstalledManifest) -ne $sourceManifestHash -or
    (Get-Sha256 -Path $AppDataManifest) -ne $sourceManifestHash) {
    throw 'Installed or AppData manifest does not match assets\manifest-cn.json.'
}
Write-Host "[DictatingMe] AppData manifest -> $AppDataManifest" -ForegroundColor Green

if ($DebugBuild) {
    $debugLog = $null
    for ($attempt = 0; $attempt -lt 80 -and $null -eq $debugLog; $attempt++) {
        $debugLog = Get-ChildItem -LiteralPath $DebugLogDir -Filter 'dictatingme-dev*.log' -File `
            -ErrorAction SilentlyContinue |
            Where-Object { $_.LastWriteTimeUtc -ge $installStartedAt.AddSeconds(-2) } |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1
        if ($null -eq $debugLog) {
            Start-Sleep -Milliseconds 250
        }
    }
    if ($null -eq $debugLog) {
        Write-Warning "The debug application is running, but its log has not refreshed yet: $DebugLogDir"
    }
    else {
        Write-Host "[DictatingMe] Debug log -> $($debugLog.FullName)" -ForegroundColor Green
    }
}
