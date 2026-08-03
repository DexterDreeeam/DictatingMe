<#
.SYNOPSIS
    Builds the release installer for one Windows architecture.

.DESCRIPTION
    Verifies the bundled KWS model, downloads the matching hash-pinned
    sherpa-onnx static library archive, builds with the Store-only Tauri
    configuration, and exports the installer plus its SHA-256.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target,

    # nsis 是发到 GitHub Release 的安装包：带卸载向导和「删除应用数据」选项，
    # 会走 runtime/windows-nsis-hooks.nsh。msi 留给 Microsoft Store 提交，
    # 它的卸载不提供任何用户数据清理入口。
    [ValidateSet('nsis', 'msi')]
    [string]$Bundle = 'nsis'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$AssetsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $AssetsDir
$CatalogPath = Join-Path $AssetsDir 'sha.json'
$MainConfigPath = Join-Path $ProjectDir 'runtime\tauri.conf.json'
$StoreConfigPath = Join-Path $ProjectDir 'runtime\tauri.microsoftstore.conf.json'
$PresetName = 'sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01'
$PresetDir = Join-Path $AssetsDir "preset\$PresetName"
$PresetArchiveUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/$PresetName.tar.bz2"
$BuildCache = Join-Path $AssetsDir '.build-cache'
$SherpaVersion = '1.13.4'

$architecture = switch ($Target) {
    'x86_64-pc-windows-msvc' { 'x64' }
    'aarch64-pc-windows-msvc' { 'arm64' }
}
$sherpaArchive = switch ($Target) {
    'x86_64-pc-windows-msvc' {
        @{
            Name = "sherpa-onnx-v$SherpaVersion-win-x64-static-MT-Release-lib.tar.bz2"
            Sha256 = 'd81bd1d25112540862d2387072e76b2b6843ef962918d6b5c7db5a19c6276b4c'
        }
    }
    'aarch64-pc-windows-msvc' {
        @{
            Name = "sherpa-onnx-v$SherpaVersion-win-arm64-static-MT-Release-lib.tar.bz2"
            Sha256 = '85504fcbe2e97b8369afe9e3ddc3c1695fe8839e9d683e42167b44174943dda1'
        }
    }
}
$sherpaArchive.Url =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$SherpaVersion/$($sherpaArchive.Name)"
$sherpaRootName = $sherpaArchive.Name.Substring(0, $sherpaArchive.Name.Length - '.tar.bz2'.Length)
$sherpaCacheDir = Join-Path $BuildCache 'sherpa-onnx'
$sherpaArchivePath = Join-Path $sherpaCacheDir $sherpaArchive.Name
$sherpaRoot = Join-Path $sherpaCacheDir $sherpaRootName
$sherpaLibDir = Join-Path $sherpaRoot 'lib'
$BundleDir = Join-Path $ProjectDir "target\$Target\release\bundle\$Bundle"
$BundleExt = if ($Bundle -eq 'nsis') { 'exe' } else { 'msi' }
$ExportDir = Join-Path $ProjectDir "release\store\$architecture"

function Assert-Command {
    param(
        [Parameter(Mandatory)]
        [string]$Name
    )

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name was not found."
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

    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Read-Utf8Json {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    $text = [System.IO.File]::ReadAllText($Path, [System.Text.UTF8Encoding]::new($false))
    $text | ConvertFrom-Json
}

function Invoke-Download {
    param(
        [Parameter(Mandatory)]
        [string]$Url,
        [Parameter(Mandatory)]
        [string]$Destination
    )

    $temporary = "$Destination.part"
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    try {
        $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
        if ($null -ne $curl) {
            & $curl.Source `
                --fail `
                --location `
                --retry 5 `
                --retry-all-errors `
                --connect-timeout 30 `
                --output $temporary `
                $Url
            if ($LASTEXITCODE -ne 0) {
                throw "curl download failed with code $LASTEXITCODE`: $Url"
            }
        }
        else {
            Invoke-WebRequest -Uri $Url -OutFile $temporary -UseBasicParsing
        }
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
    }
    finally {
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    }
}

function Ensure-Preset {
    $catalog = Read-Utf8Json -Path $CatalogPath
    $preset = @($catalog.assets | Where-Object { $_.id -eq 'evoke.sherpa-zipformer-wenetspeech' })
    if ($preset.Count -ne 1) {
        throw 'Asset catalog must contain exactly one bundled KWS preset.'
    }

    $valid = Test-Path -LiteralPath $PresetDir -PathType Container
    if ($valid) {
        foreach ($expected in $preset[0].files) {
            $path = Join-Path $PresetDir $expected.path
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                $valid = $false
                break
            }
        }
    }

    if (-not $valid) {
        Write-Host '[DictatingMe] Downloading bundled KWS model...' -ForegroundColor Cyan
        New-Item -ItemType Directory -Path $BuildCache -Force | Out-Null
        $archive = Join-Path $BuildCache "$PresetName.tar.bz2"
        Invoke-Download -Url $PresetArchiveUrl -Destination $archive
        $presetParent = Split-Path -Parent $PresetDir
        New-Item -ItemType Directory -Path $presetParent -Force | Out-Null
        Remove-Item -LiteralPath $PresetDir -Recurse -Force -ErrorAction SilentlyContinue
        & tar.exe -xjf $archive -C $presetParent
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to extract bundled KWS model with code $LASTEXITCODE."
        }
    }

    foreach ($expected in $preset[0].files) {
        $path = Join-Path $PresetDir $expected.path
        Assert-File -Path $path -Description "Bundled KWS file $($expected.path)"
        $item = Get-Item -LiteralPath $path
        if ($item.Length -ne [long]$expected.sizeBytes) {
            throw "Bundled KWS size mismatch: $path"
        }
        if ((Get-Sha256 -Path $path) -ne [string]$expected.sha256) {
            throw "Bundled KWS SHA-256 mismatch: $path"
        }
    }
}

function Ensure-SherpaLibraries {
    if (Test-Path -LiteralPath $sherpaLibDir -PathType Container) {
        return
    }

    New-Item -ItemType Directory -Path $sherpaCacheDir -Force | Out-Null
    if (-not (Test-Path -LiteralPath $sherpaArchivePath -PathType Leaf) -or
        (Get-Sha256 -Path $sherpaArchivePath) -ne $sherpaArchive.Sha256) {
        Write-Host "[DictatingMe] Downloading sherpa-onnx $architecture static libraries..." `
            -ForegroundColor Cyan
        Remove-Item -LiteralPath $sherpaArchivePath -Force -ErrorAction SilentlyContinue
        Invoke-Download -Url $sherpaArchive.Url -Destination $sherpaArchivePath
    }
    if ((Get-Sha256 -Path $sherpaArchivePath) -ne $sherpaArchive.Sha256) {
        throw "sherpa-onnx archive SHA-256 mismatch: $sherpaArchivePath"
    }

    Remove-Item -LiteralPath $sherpaRoot -Recurse -Force -ErrorAction SilentlyContinue
    & tar.exe -xjf $sherpaArchivePath -C $sherpaCacheDir
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to extract sherpa-onnx libraries with code $LASTEXITCODE."
    }
    if (-not (Test-Path -LiteralPath $sherpaLibDir -PathType Container)) {
        throw "sherpa-onnx archive does not contain the expected lib directory: $sherpaLibDir"
    }
}

function Initialize-Arm64MsvcEnvironment {
    if ($Target -ne 'aarch64-pc-windows-msvc') {
        return
    }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw 'Visual Studio Installer vswhere.exe is required for the ARM64 build.'
    }
    $installationPath = (& $vswhere `
        -latest `
        -products '*' `
        -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 `
        -property installationPath |
        Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($installationPath)) {
        throw 'MSVC ARM64 build tools are not installed.'
    }
    $vcvars = Join-Path $installationPath 'VC\Auxiliary\Build\vcvarsall.bat'
    Assert-File -Path $vcvars -Description 'Visual Studio vcvarsall.bat'
    $vswhereDirectory = Split-Path -Parent $vswhere
    if (($env:Path -split ';') -notcontains $vswhereDirectory) {
        $env:Path = "$vswhereDirectory;$env:Path"
    }
    $vcvarsArchitecture = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
        'arm64'
    }
    else {
        'amd64_arm64'
    }
    $environment = & $env:ComSpec /s /c "`"$vcvars`" $vcvarsArchitecture >nul && set"
    if ($LASTEXITCODE -ne 0) {
        throw "vcvarsall $vcvarsArchitecture failed with code $LASTEXITCODE."
    }
    foreach ($line in $environment) {
        $separator = $line.IndexOf('=')
        if ($separator -le 0) {
            continue
        }
        $name = $line.Substring(0, $separator)
        $value = $line.Substring($separator + 1)
        [System.Environment]::SetEnvironmentVariable($name, $value, 'Process')
    }
}

Write-Host "=== DictatingMe release $Bundle ($architecture) ===" `
    -ForegroundColor Magenta
Write-Host "Target -> $Target"
Write-Host "Publisher -> Dexter Tsou"
Write-Host ''

Assert-Command -Name 'npm.cmd'
Assert-Command -Name 'cargo.exe'
Assert-Command -Name 'rustup.exe'
Assert-Command -Name 'tar.exe'
Assert-File -Path $CatalogPath -Description 'Asset SHA catalog'
Assert-File -Path $MainConfigPath -Description 'Tauri configuration'
Assert-File -Path $StoreConfigPath -Description 'Microsoft Store Tauri configuration'

if (-not (Test-Path -LiteralPath (Join-Path $ProjectDir 'node_modules') -PathType Container)) {
    Push-Location $ProjectDir
    try {
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) {
            throw "npm ci failed with code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

Ensure-Preset
Ensure-SherpaLibraries
Initialize-Arm64MsvcEnvironment

& rustup.exe target add $Target
if ($LASTEXITCODE -ne 0) {
    throw "rustup target add failed with code $LASTEXITCODE."
}

$previousSherpaLibDir = $env:SHERPA_ONNX_LIB_DIR
$env:SHERPA_ONNX_LIB_DIR = $sherpaLibDir

# MSI 走 Store 专用配置；NSIS 必须用主配置——Store 配置把 bundle.targets 换成
# msi，且它的 bundle.windows 不含 nsis.installerHooks，用它构建出来的 NSIS 包
# 会丢掉卸载时清理用户数据的 hook。
$configArgs = if ($Bundle -eq 'msi') { @('--config', $StoreConfigPath) } else { @() }

Push-Location $ProjectDir
try {
    & npm.cmd run tauri -- build `
        --target $Target `
        --bundles $Bundle `
        @configArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri $Bundle build exited with code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
    $env:SHERPA_ONNX_LIB_DIR = $previousSherpaLibDir
}

$installers = @(
    Get-ChildItem -LiteralPath $BundleDir -Filter "*.$BundleExt" -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending
)
if ($installers.Count -eq 0) {
    throw "No $Bundle installer was produced in $BundleDir"
}

$config = Read-Utf8Json -Path $MainConfigPath
$version = [string]$config.version
New-Item -ItemType Directory -Path $ExportDir -Force | Out-Null
Get-ChildItem -LiteralPath $ExportDir -File -ErrorAction SilentlyContinue |
    Remove-Item -Force
$exported = Join-Path $ExportDir "DictatingMe_${version}_${architecture}.$BundleExt"
Copy-Item -LiteralPath $installers[0].FullName -Destination $exported

$signature = Get-AuthenticodeSignature -LiteralPath $exported
if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
    throw "Build unexpectedly carries a signature ($($signature.Status)): $exported"
}

$hash = Get-Sha256 -Path $exported
$hashFile = "$exported.sha256"
[System.IO.File]::WriteAllText(
    $hashFile,
    "$hash  $([System.IO.Path]::GetFileName($exported))`r`n",
    [System.Text.UTF8Encoding]::new($false)
)

Write-Host ''
Write-Host "[DictatingMe] Release $Bundle completed." -ForegroundColor Green
Write-Host "Installer -> $exported" -ForegroundColor Green
Write-Host "SHA-256 -> $hash" -ForegroundColor Green
