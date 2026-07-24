<#
.SYNOPSIS
    Downloads the local ONNX models required by DictatingMe (DictationModel + EvokeModel).

.DESCRIPTION
    Model files are large third-party release artifacts and should not be committed to git:
    everything in this directory (model/) except this script itself is excluded via the
    root .gitignore. Run this script once when setting up a dev environment or CI build to
    fetch the models into model/dictation and model/evoke.

    Model choices (placeholders for the brainstorm stage, see brainstrom/plan.md section 3.1):

    - DictationModel -> Alibaba DAMO Academy SenseVoice (official ONNX export, hosted on the
      sherpa-onnx release). Note: plan.md originally considered "Qwen ASR", but the Qwen
      audio models currently have no official ONNX export suited for lightweight local
      streaming deployment. SenseVoice is also from Alibaba, purpose-built for local/
      streaming recognition, and already ships as ready-to-use ONNX, so it is used as the
      current placeholder. If the model choice changes later, just edit $DictationModelUrl
      below.

    - EvokeModel -> the official Chinese keyword-spotting (KWS) model from sherpa-onnx,
      3.3M parameters, sized to fit plan.md's "<20MB, resident in memory" target, used as a
      working baseline. plan.md's "fine-tune the wake-word model with the user's own voice"
      is a long-term goal (see EvokeModelEngine::fine_tune_with_voice_samples); this gives a
      ready-to-use model so the Listening state can actually run.

.PARAMETER Force
    If the destination directory already has content, delete it and re-download.

.EXAMPLE
    ./download-models.ps1
.EXAMPLE
    ./download-models.ps1 -Force
#>

[CmdletBinding()]
param(
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest renders a progress bar by default, which has a noticeable performance
# cost for large downloads, so turn it off.
$ProgressPreference = 'SilentlyContinue'

$RootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$DictationDir = Join-Path $RootDir 'dictation'
$EvokeDir = Join-Path $RootDir 'evoke'

# --- Model config: once the final model choice is confirmed, only these two URLs/names need to change ---
$DictationModelUrl = 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2'
$DictationModelArchiveName = 'sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2'

$EvokeModelUrl = 'https://github.com/pkufool/keyword-spotting-models/releases/download/v0.1/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz'
$EvokeModelArchiveName = 'sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz'
# ----------------------------------------------------------------------------------------------------

function Get-AndExtractModel {
    param(
        [string]$Name,
        [string]$Url,
        [string]$ArchiveFileName,
        [string]$DestDir
    )

    if ((Test-Path $DestDir) -and (Get-ChildItem $DestDir -ErrorAction SilentlyContinue)) {
        if (-not $Force) {
            Write-Host "[$Name] Destination already has content, skipping download (use -Force to re-download): $DestDir" -ForegroundColor Yellow
            return
        }
        Write-Host "[$Name] -Force specified, clearing old directory: $DestDir" -ForegroundColor Yellow
        Remove-Item -Recurse -Force $DestDir
    }

    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null

    $archivePath = Join-Path $DestDir $ArchiveFileName
    Write-Host "[$Name] Downloading <- $Url" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $Url -OutFile $archivePath -UseBasicParsing

    Write-Host "[$Name] Extracting -> $archivePath" -ForegroundColor Cyan
    # .tar.bz2 / .tar.bz are both bzip2-compressed tar archives; the tar.exe (bsdtar) bundled
    # with Windows 10 1803+ / Windows 11 natively supports -j (bzip2), no extra tools needed.
    tar -xjf $archivePath -C $DestDir
    if ($LASTEXITCODE -ne 0) {
        throw "[$Name] Extraction failed: make sure your system tar supports bzip2 (bundled with Windows 10 1803+ / Windows 11), or extract $archivePath manually with 7-Zip."
    }

    Remove-Item $archivePath -Force
    Write-Host "[$Name] Done -> $DestDir" -ForegroundColor Green
}

Write-Host '=== DictatingMe model download ===' -ForegroundColor Magenta

Get-AndExtractModel -Name 'DictationModel (SenseVoice)' -Url $DictationModelUrl -ArchiveFileName $DictationModelArchiveName -DestDir $DictationDir
Get-AndExtractModel -Name 'EvokeModel (KWS)' -Url $EvokeModelUrl -ArchiveFileName $EvokeModelArchiveName -DestDir $EvokeDir

Write-Host ''
Write-Host 'All done:' -ForegroundColor Magenta
Write-Host "  DictationModel -> $DictationDir"
Write-Host "  EvokeModel     -> $EvokeDir"
