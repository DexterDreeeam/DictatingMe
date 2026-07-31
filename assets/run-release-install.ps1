<#
.SYNOPSIS
    Builds the DictatingMe Release NSIS installer, exports it, then silently
    installs it over the existing installation.

.DESCRIPTION
    Same production build and export path as run-release.ps1, but does not stop
    at the installer: it also performs the silent overwrite that run-package.ps1
    normally does, then verifies and launches the installed application.

    In other words this is run_release.cmd followed by the installation half of
    run_install.cmd, without building twice.
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

# 去掉 -BuildOnly 就会继续走静默覆盖安装；-ExportDirectory 保留，
# 让 release\ 下依然留有和 run_release.cmd 一致的安装包副本。
& $PackageScript -ExportDirectory $ReleaseDir
