@echo off
REM Regenerate the voice-over used by promo-footage.html.
REM Uses edge-tts (Microsoft Edge read-aloud endpoint: free, no API key).
REM Output goes to brainstrom\promo-voice\ which is gitignored.
REM
REM The spoken text lives in promo-voice\lines\*.txt as UTF-8 files rather than
REM inline here: cmd.exe decodes .cmd files with the ANSI codepage, which
REM mangles non-ASCII arguments before they ever reach python.
setlocal
cd /d "%~dp0.."

set VOICE=zh-CN-XiaoxiaoNeural
set RATE=+25%%
set OUT=brainstrom\promo-voice

python -c "import edge_tts" 2>nul || (
  echo Installing edge-tts...
  python -m pip install edge-tts --quiet --disable-pip-version-check || goto :fail
)

for %%N in (wake sentence) do (
  echo Synthesizing %%N ...
  python -m edge_tts --voice %VOICE% --rate "%RATE%" ^
    --file "%OUT%\lines\%%N.txt" --write-media "%OUT%\%%N.mp3" || goto :fail
)

echo.
echo Done: %OUT%  ^(voice=%VOICE% rate=%RATE%^)
exit /b 0

:fail
echo.
echo Failed. Needs Python 3 and network access.
echo Other voices: zh-CN-XiaoyiNeural / zh-CN-YunxiNeural / zh-CN-YunyangNeural
exit /b 1