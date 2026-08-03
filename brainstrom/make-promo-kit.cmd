@echo off
REM Bundle promo-footage.html into a self-contained folder that can be copied
REM and shared as-is. Output: brainstrom\promo-kit\
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0make-promo-kit.ps1"
if errorlevel 1 exit /b 1
exit /b 0