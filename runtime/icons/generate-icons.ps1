$ErrorActionPreference = "Stop"

$iconDirectory = $PSScriptRoot
$repositoryRoot = Split-Path (Split-Path $iconDirectory -Parent) -Parent
$sourceIcon = Join-Path $iconDirectory "app-icon.svg"
$wordmark = Join-Path $iconDirectory "wordmark-d.png"
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) "dictatingme-icons-$([guid]::NewGuid().ToString('N'))"

function Invoke-TauriIcon {
    param(
        [Parameter(Mandatory = $true)]
        [string]$InputPath,
        [Parameter(Mandatory = $true)]
        [string]$OutputPath,
        [int[]]$PngSizes = @()
    )

    New-Item -ItemType Directory -Force -Path $OutputPath | Out-Null
    $arguments = @("run", "tauri", "--", "icon", $InputPath, "-o", $OutputPath)
    foreach ($size in $PngSizes) {
        $arguments += @("-p", $size)
    }

    & npm @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri icon generation failed with exit code $LASTEXITCODE."
    }
}

Push-Location $repositoryRoot
try {
    $neutralDirectory = Join-Path $temporaryRoot "neutral"
    Invoke-TauriIcon -InputPath $sourceIcon -OutputPath $neutralDirectory

    foreach ($name in @("32x32.png", "128x128.png", "128x128@2x.png", "icon.ico")) {
        Copy-Item (Join-Path $neutralDirectory $name) (Join-Path $iconDirectory $name) -Force
    }
    Copy-Item (Join-Path $neutralDirectory "32x32.png") (Join-Path $iconDirectory "tray.png") -Force

    $sourceMarkup = Get-Content $sourceIcon -Raw
    $themeColors = [ordered]@{
        dark = "#f5f6f7"
        light = "#4f535b"
    }

    foreach ($theme in $themeColors.Keys) {
        $themeDirectory = Join-Path $temporaryRoot $theme
        New-Item -ItemType Directory -Force -Path $themeDirectory | Out-Null
        Copy-Item $wordmark (Join-Path $themeDirectory "wordmark-d.png")

        $themeSource = Join-Path $themeDirectory "app-icon.svg"
        $sourceMarkup.Replace("#7a7e86", $themeColors[$theme]) |
            Set-Content $themeSource -Encoding utf8 -NoNewline

        Invoke-TauriIcon -InputPath $themeSource -OutputPath $themeDirectory -PngSizes @(256, 32)
        Copy-Item (Join-Path $themeDirectory "256x256.png") (Join-Path $iconDirectory "logo-$theme.png") -Force
        Copy-Item (Join-Path $themeDirectory "32x32.png") (Join-Path $iconDirectory "tray-$theme.png") -Force
    }
}
finally {
    Pop-Location
    Remove-Item $temporaryRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "DictatingMe icon assets regenerated."
