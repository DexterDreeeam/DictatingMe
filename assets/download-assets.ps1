<#
.SYNOPSIS
    Verifies and downloads all local DictatingMe models and acceptance-test audio assets.

.DESCRIPTION
    The script validates extracted model files instead of treating a non-empty directory as
    complete. Missing or invalid model packages are downloaded and extracted again.

    It also validates and downloads 30 selected 16 kHz MS-SNSD background-noise WAV files
    into assets/noise for later wake-word false-positive acceptance testing.

    Downloaded files are excluded from git; the PowerShell script and root CMD launcher are tracked.

.PARAMETER Force
    Re-download and replace all model packages and noise assets even when validation succeeds.

.EXAMPLE
    ./download-assets.ps1
.EXAMPLE
    ./download-assets.ps1 -Force
#>

[CmdletBinding()]
param(
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$RootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$DictationDir = Join-Path $RootDir 'dictation'
$PresetDir = Join-Path $RootDir 'preset'
$NoiseDir = Join-Path $RootDir 'noise'

$DictationPackage = 'sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20'
$DictationArchive = "$DictationPackage.tar.bz2"
$DictationUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/$DictationArchive"
$DictationRoot = Join-Path $DictationDir $DictationPackage
$DictationRequired = @(
    @{ Path = 'encoder-epoch-99-avg-1.int8.onnx'; MinBytes = 100000 },
    @{ Path = 'decoder-epoch-99-avg-1.onnx'; MinBytes = 100000 },
    @{ Path = 'joiner-epoch-99-avg-1.int8.onnx'; MinBytes = 100000 },
    @{ Path = 'tokens.txt'; MinBytes = 1000 }
)

$EvokePackage = 'sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01'
$EvokeArchive = "$EvokePackage.tar.bz2"
$EvokeUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/$EvokeArchive"
$EvokeRoot = Join-Path $PresetDir $EvokePackage
$EvokeRequired = @(
    @{ Path = 'encoder-epoch-12-avg-2-chunk-16-left-64.onnx'; MinBytes = 100000 },
    @{ Path = 'decoder-epoch-12-avg-2-chunk-16-left-64.onnx'; MinBytes = 100000 },
    @{ Path = 'joiner-epoch-12-avg-2-chunk-16-left-64.onnx'; MinBytes = 100000 },
    @{ Path = 'tokens.txt'; MinBytes = 1000 }
)

$NoiseCommit = 'fe61c4ba0d9ac8dd7e23d719cc79f8947e1dc742'
$NoiseBaseUrl = "https://raw.githubusercontent.com/microsoft/MS-SNSD/$NoiseCommit"
$NoiseAssets = @(
    @{ Name = 'environment-air-conditioner-train.wav'; Source = 'noise_train/AirConditioner_1.wav' },
    @{ Name = 'crowd-airport-announcements-train.wav'; Source = 'noise_train/AirportAnnouncements_1.wav' },
    @{ Name = 'crowd-babble-train-1.wav'; Source = 'noise_train/Babble_1.wav' },
    @{ Name = 'crowd-babble-train-2.wav'; Source = 'noise_train/Babble_2.wav' },
    @{ Name = 'crowd-babble-train-3.wav'; Source = 'noise_train/Babble_3.wav' },
    @{ Name = 'traffic-bus-train.wav'; Source = 'noise_train/Bus_1.wav' },
    @{ Name = 'crowd-cafeteria-train.wav'; Source = 'noise_train/CafeTeria_1.wav' },
    @{ Name = 'crowd-cafe-train.wav'; Source = 'noise_train/Cafe_1.wav' },
    @{ Name = 'traffic-car-train.wav'; Source = 'noise_train/Car_1.wav' },
    @{ Name = 'machinery-copy-machine-train-1.wav'; Source = 'noise_train/CopyMachine_1.wav' },
    @{ Name = 'machinery-copy-machine-train-2.wav'; Source = 'noise_train/CopyMachine_2.wav' },
    @{ Name = 'machinery-copy-machine-train-3.wav'; Source = 'noise_train/CopyMachine_3.wav' },
    @{ Name = 'traffic-metro-train.wav'; Source = 'noise_train/Metro_1.wav' },
    @{ Name = 'traffic-station-train.wav'; Source = 'noise_train/Station_1.wav' },
    @{ Name = 'traffic-road-train.wav'; Source = 'noise_train/Traffic_1.wav' },
    @{ Name = 'office-typing-train-1.wav'; Source = 'noise_train/Typing_1.wav' },
    @{ Name = 'office-typing-train-2.wav'; Source = 'noise_train/Typing_2.wav' },
    @{ Name = 'office-typing-train-3.wav'; Source = 'noise_train/Typing_3.wav' },
    @{ Name = 'machinery-vacuum-cleaner-train-1.wav'; Source = 'noise_train/VacuumCleaner_1.wav' },
    @{ Name = 'machinery-vacuum-cleaner-train-2.wav'; Source = 'noise_train/VacuumCleaner_2.wav' },
    @{ Name = 'machinery-vacuum-cleaner-train-3.wav'; Source = 'noise_train/VacuumCleaner_3.wav' },
    @{ Name = 'home-washing-machine-train.wav'; Source = 'noise_train/Washing_1.wav' },
    @{ Name = 'crowd-babble-test-1.wav'; Source = 'noise_test/Babble_1.wav' },
    @{ Name = 'crowd-babble-test-2.wav'; Source = 'noise_test/Babble_2.wav' },
    @{ Name = 'crowd-babble-test-3.wav'; Source = 'noise_test/Babble_3.wav' },
    @{ Name = 'machinery-copy-machine-test-1.wav'; Source = 'noise_test/CopyMachine_1.wav' },
    @{ Name = 'machinery-copy-machine-test-2.wav'; Source = 'noise_test/CopyMachine_2.wav' },
    @{ Name = 'office-typing-test-1.wav'; Source = 'noise_test/Typing_1.wav' },
    @{ Name = 'office-typing-test-2.wav'; Source = 'noise_test/Typing_2.wav' },
    @{ Name = 'machinery-vacuum-cleaner-test-1.wav'; Source = 'noise_test/VacuumCleaner_1.wav' }
)

$LegacyNoiseAssets = @{
    'crowd-babble.wav' = 'crowd-babble-test-1.wav'
    'crowd-cafeteria.wav' = 'crowd-cafeteria-train.wav'
    'traffic-road.wav' = 'traffic-road-train.wav'
    'traffic-metro.wav' = 'traffic-metro-train.wav'
    'machinery-copy-machine.wav' = 'machinery-copy-machine-test-1.wav'
    'machinery-vacuum-cleaner.wav' = 'machinery-vacuum-cleaner-test-1.wav'
}

function Test-RequiredFiles {
    param(
        [Parameter(Mandatory)]
        [string]$BaseDir,
        [Parameter(Mandatory)]
        [array]$RequiredFiles
    )

    foreach ($requirement in $RequiredFiles) {
        $path = Join-Path $BaseDir $requirement.Path
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            return $false
        }
        if ((Get-Item -LiteralPath $path).Length -lt [long]$requirement.MinBytes) {
            return $false
        }
    }
    return $true
}

function Test-WavAsset {
    param(
        [Parameter(Mandatory)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }
    if ((Get-Item -LiteralPath $Path).Length -lt 100000) {
        return $false
    }

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        if ($stream.Length -lt 44) {
            return $false
        }
        $reader = [System.IO.BinaryReader]::new($stream)
        try {
            $riff = [System.Text.Encoding]::ASCII.GetString($reader.ReadBytes(4))
            $null = $reader.ReadInt32()
            $wave = [System.Text.Encoding]::ASCII.GetString($reader.ReadBytes(4))
            $stream.Position = 24
            $sampleRate = $reader.ReadInt32()
            return $riff -eq 'RIFF' -and $wave -eq 'WAVE' -and $sampleRate -eq 16000
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Invoke-Download {
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [Parameter(Mandatory)]
        [string]$Url,
        [Parameter(Mandatory)]
        [string]$Destination
    )

    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $partial = "$Destination.part"
    Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue

    $lastError = $null
    for ($attempt = 1; $attempt -le 3; $attempt++) {
        try {
            Write-Host "[$Name] Download attempt $attempt/3 <- $Url" -ForegroundColor Cyan
            Invoke-WebRequest -Uri $Url -OutFile $partial -UseBasicParsing
            if ((Get-Item -LiteralPath $partial).Length -eq 0) {
                throw 'Downloaded file is empty.'
            }
            Move-Item -LiteralPath $partial -Destination $Destination -Force
            return
        }
        catch {
            $lastError = $_
            Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue
            if ($attempt -lt 3) {
                Start-Sleep -Seconds (2 * $attempt)
            }
        }
    }

    if ($Url -match '^https://github\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/([^/]+)$' -and
        (Get-Command gh -ErrorAction SilentlyContinue)) {
        $repo = "$($Matches[1])/$($Matches[2])"
        $tag = $Matches[3]
        $asset = $Matches[4]
        $temporaryDir = Join-Path $parent ".gh-$([guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Directory -Force -Path $temporaryDir | Out-Null
        try {
            Write-Host "[$Name] Falling back to GitHub CLI." -ForegroundColor Yellow
            gh release download $tag --repo $repo --pattern $asset --dir $temporaryDir
            if ($LASTEXITCODE -ne 0) {
                throw "GitHub CLI exited with code $LASTEXITCODE."
            }
            Move-Item -LiteralPath (Join-Path $temporaryDir $asset) -Destination $Destination -Force
            return
        }
        finally {
            Remove-Item -LiteralPath $temporaryDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    throw "[$Name] Download failed after 3 attempts: $($lastError.Exception.Message)"
}

function Ensure-ModelPackage {
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [Parameter(Mandatory)]
        [string]$Url,
        [Parameter(Mandatory)]
        [string]$ArchiveName,
        [Parameter(Mandatory)]
        [string]$DestinationDir,
        [Parameter(Mandatory)]
        [string]$ExpectedRoot,
        [Parameter(Mandatory)]
        [array]$RequiredFiles
    )

    if (-not $Force -and (Test-RequiredFiles -BaseDir $ExpectedRoot -RequiredFiles $RequiredFiles)) {
        Write-Host "[$Name] Verified extracted model -> $ExpectedRoot" -ForegroundColor Green
        return
    }

    if (Test-Path -LiteralPath $ExpectedRoot) {
        Write-Host "[$Name] Existing package is incomplete or forced; clearing its package directory." -ForegroundColor Yellow
        Remove-Item -LiteralPath $ExpectedRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $DestinationDir | Out-Null

    $archivePath = Join-Path $DestinationDir $ArchiveName
    try {
        Invoke-Download -Name $Name -Url $Url -Destination $archivePath
        Write-Host "[$Name] Extracting -> $DestinationDir" -ForegroundColor Cyan
        tar -xjf $archivePath -C $DestinationDir
        if ($LASTEXITCODE -ne 0) {
            throw "tar exited with code $LASTEXITCODE."
        }
    }
    finally {
        Remove-Item -LiteralPath $archivePath -Force -ErrorAction SilentlyContinue
    }

    if (-not (Test-RequiredFiles -BaseDir $ExpectedRoot -RequiredFiles $RequiredFiles)) {
        Remove-Item -LiteralPath $ExpectedRoot -Recurse -Force -ErrorAction SilentlyContinue
        throw "[$Name] Extraction completed but required model files are missing or invalid."
    }
    Write-Host "[$Name] Downloaded, extracted, and verified -> $ExpectedRoot" -ForegroundColor Green
}

function Ensure-NoiseAsset {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Asset
    )

    $destination = Join-Path $NoiseDir $Asset.Name
    if (-not $Force -and (Test-WavAsset -Path $destination)) {
        Write-Host "[Noise asset] Verified -> $destination" -ForegroundColor Green
        return
    }

    Remove-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue
    $url = "$NoiseBaseUrl/$($Asset.Source)"
    Invoke-Download -Name "Noise asset $($Asset.Name)" -Url $url -Destination $destination
    if (-not (Test-WavAsset -Path $destination)) {
        Remove-Item -LiteralPath $destination -Force -ErrorAction SilentlyContinue
        throw "[Noise asset] Downloaded WAV failed RIFF/WAVE/16kHz validation: $($Asset.Name)"
    }
    Write-Host "[Noise asset] Downloaded and verified -> $destination" -ForegroundColor Green
}

Write-Host '=== DictatingMe asset verification ===' -ForegroundColor Magenta

Ensure-ModelPackage `
    -Name 'DictationModel (Streaming Zipformer)' `
    -Url $DictationUrl `
    -ArchiveName $DictationArchive `
    -DestinationDir $DictationDir `
    -ExpectedRoot $DictationRoot `
    -RequiredFiles $DictationRequired

Ensure-ModelPackage `
    -Name 'EvokeModel (KWS)' `
    -Url $EvokeUrl `
    -ArchiveName $EvokeArchive `
    -DestinationDir $PresetDir `
    -ExpectedRoot $EvokeRoot `
    -RequiredFiles $EvokeRequired

New-Item -ItemType Directory -Force -Path $NoiseDir | Out-Null
foreach ($legacyName in $LegacyNoiseAssets.Keys) {
    $legacyPath = Join-Path $RootDir $legacyName
    $destination = Join-Path $NoiseDir $LegacyNoiseAssets[$legacyName]
    if (-not $Force -and (Test-WavAsset -Path $legacyPath) -and -not (Test-Path -LiteralPath $destination)) {
        Move-Item -LiteralPath $legacyPath -Destination $destination
        Write-Host "[Noise asset] Migrated legacy file -> $destination" -ForegroundColor Green
    }
    else {
        Remove-Item -LiteralPath $legacyPath -Force -ErrorAction SilentlyContinue
    }
}
foreach ($asset in $NoiseAssets) {
    Ensure-NoiseAsset -Asset $asset
}

Write-Host ''
Write-Host 'All required models and assets are ready:' -ForegroundColor Magenta
Write-Host "  DictationModel -> $DictationRoot"
Write-Host "  Preset Evoke   -> $EvokeRoot"
Write-Host "  Noise assets   -> $NoiseDir ($($NoiseAssets.Count) files)"
Write-Host ''
Write-Host 'MS-SNSD noise sources: CC0 Freesound and CC BY-SA 3.0 DEMAND; see the MS-SNSD README for attribution details.'
