@echo off
setlocal

cd /d "%~dp0"
if errorlevel 1 (
  echo [DictatingMe] Failed to enter the project directory.
  pause
  exit /b 1
)

if not exist "%~dp0assets\download-assets.ps1" (
  echo [DictatingMe] Asset download script is missing: assets\download-assets.ps1
  pause
  exit /b 1
)

powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0assets\download-assets.ps1" %*
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [DictatingMe] Asset preparation failed with code %EXIT_CODE%.
) else (
  echo.
  echo [DictatingMe] Asset preparation completed successfully.
)

echo [DictatingMe] Press any key to close this window.
pause >nul

exit /b %EXIT_CODE%
