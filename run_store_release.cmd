@echo off
setlocal

cd /d "%~dp0"
if errorlevel 1 (
  echo [DictatingMe] Failed to enter the project directory.
  pause
  exit /b 1
)

if not exist "%~dp0assets\run-store-release.ps1" (
  echo [DictatingMe] Store release script is missing: assets\run-store-release.ps1
  pause
  exit /b 1
)

set "ARCH=%~1"
if "%ARCH%"=="" set "ARCH=all"

if /I "%ARCH%"=="x64" goto build_x64
if /I "%ARCH%"=="amd64" goto build_x64
if /I "%ARCH%"=="arm64" goto build_arm64
if /I "%ARCH%"=="all" goto build_all

echo [DictatingMe] Usage: run_store_release.cmd [all^|x64^|amd64^|arm64]
pause
exit /b 2

:build_all
call :run_target x86_64-pc-windows-msvc
if errorlevel 1 goto failed
call :run_target aarch64-pc-windows-msvc
if errorlevel 1 goto failed
goto success

:build_x64
call :run_target x86_64-pc-windows-msvc
if errorlevel 1 goto failed
goto success

:build_arm64
call :run_target aarch64-pc-windows-msvc
if errorlevel 1 goto failed
goto success

:run_target
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass ^
  -File "%~dp0assets\run-store-release.ps1" -Target "%~1" -Bundle msi
exit /b %ERRORLEVEL%

:failed
set "EXIT_CODE=%ERRORLEVEL%"
echo.
echo [DictatingMe] Store MSI build failed with code %EXIT_CODE%.
echo [DictatingMe] Press any key to close this window.
pause >nul
exit /b %EXIT_CODE%

:success
echo.
echo [DictatingMe] Store MSI exported to: %~dp0release\store\
echo [DictatingMe] Press any key to close this window.
pause >nul
exit /b 0
