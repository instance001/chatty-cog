@echo off
setlocal

cd /d "%~dp0"
set "CHATTYCOG_BASE_PATH=%~dp0"

if not exist "chattycog_gui.exe" (
    echo Missing chattycog_gui.exe in %~dp0
    pause
    exit /b 1
)

start "" "chattycog_gui.exe"
