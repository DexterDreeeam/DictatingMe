@echo off
setlocal

fltmc >nul 2>&1
if errorlevel 1 (
  echo [DictatingMe] Administrator privileges are required for elevated foreground applications.
  echo [DictatingMe] Requesting elevation...
  powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -Verb RunAs -WorkingDirectory '%~dp0'"
  exit /b
)

cd /d "%~dp0"
if errorlevel 1 (
  echo [DictatingMe] Failed to enter the project directory.
  pause
  exit /b 1
)

where npm.cmd >nul 2>&1
if errorlevel 1 (
  echo [DictatingMe] npm.cmd was not found. Install Node.js first.
  pause
  exit /b 1
)

if not exist "node_modules\" (
  echo [DictatingMe] Dependencies are missing.
  echo Run: npm.cmd install --registry=https://repo.huaweicloud.com/repository/npm/
  pause
  exit /b 1
)

if not exist "assets\preset\sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01\" (
  echo [DictatingMe] The wake-word model is missing.
  echo Run: download-assets.cmd
  pause
  exit /b 1
)

if not exist "assets\sha.json" (
  echo [DictatingMe] The asset SHA catalog is missing.
  pause
  exit /b 1
)

set "SHERPA_ONNX_ARCHIVE_DIR=%LOCALAPPDATA%\DictatingMe\sherpa-cache"
set "DICTATINGME_DEV_LOG_DIR=%~dp0logs"
if not exist "%DICTATINGME_DEV_LOG_DIR%\" (
  mkdir "%DICTATINGME_DEV_LOG_DIR%"
  if errorlevel 1 (
    echo [DictatingMe] Failed to create the development log directory.
    pause
    exit /b 1
  )
)

echo [DictatingMe] Starting development mode...
echo [DictatingMe] Integrity level: Administrator
echo [DictatingMe] Development log directory: %DICTATINGME_DEV_LOG_DIR%
echo [DictatingMe] Development log files: %DICTATINGME_DEV_LOG_DIR%\dictatingme-dev.YYYY-MM-DD.log
echo [DictatingMe] Press Ctrl+C to stop.
echo.

call npm.cmd run tauri dev
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [DictatingMe] Development process exited with code %EXIT_CODE%.
  pause
)

exit /b %EXIT_CODE%
