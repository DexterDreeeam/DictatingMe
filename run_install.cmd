@echo off
setlocal

fltmc >nul 2>&1
if errorlevel 1 (
  echo [DictatingMe] Administrator privileges are required for a silent per-machine installation.
  echo [DictatingMe] Requesting elevation...
  powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -Verb RunAs -WorkingDirectory '%~dp0'"
  exit /b
)

cd /d "%~dp0"
if errorlevel 1 (
  echo [DictatingMe] Failed to enter the project directory.
  pause
  exit /b 1
)

if not exist "%~dp0assets\run-package.ps1" (
  echo [DictatingMe] Package script is missing: assets\run-package.ps1
  pause
  exit /b 1
)

powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0assets\run-package.ps1" %*
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [DictatingMe] Package build or silent installation failed with code %EXIT_CODE%.
) else (
  echo.
  echo [DictatingMe] Package build and silent installation completed successfully.
)

echo [DictatingMe] Press any key to close this window.
pause >nul

exit /b %EXIT_CODE%
