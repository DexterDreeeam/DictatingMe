@echo off
setlocal

cd /d "%~dp0"
if errorlevel 1 (
  echo [DictatingMe] Failed to enter the project directory.
  pause
  exit /b 1
)

if not exist "%~dp0assets\run-release.ps1" (
  echo [DictatingMe] Release script is missing: assets\run-release.ps1
  pause
  exit /b 1
)

powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0assets\run-release.ps1"
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [DictatingMe] Release build failed with code %EXIT_CODE%.
) else (
  echo.
  echo [DictatingMe] Release installer exported to: %~dp0release\
)

echo [DictatingMe] Press any key to close this window.
pause >nul

exit /b %EXIT_CODE%
