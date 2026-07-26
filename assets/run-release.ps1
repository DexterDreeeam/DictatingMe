<#
.SYNOPSIS
    Builds and exports the DictatingMe Release NSIS installer.

.DESCRIPTION
    Reuses the package validation and production build path, but never stops,
    installs, or launches DictatingMe. The latest installer is exported to the
    repository's ignored release directory.
#>

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$AssetsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $AssetsDir
$PackageScript = Join-Path $AssetsDir 'run-package.ps1'
$ReleaseDir = Join-Path $ProjectDir 'release'

if (-not (Test-Path -LiteralPath $PackageScript -PathType Leaf)) {
    throw "Package script is missing: $PackageScript"
}

& $PackageScript -BuildOnly -ExportDirectory $ReleaseDir
