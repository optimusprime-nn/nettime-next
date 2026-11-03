@echo off
cd /d "%~dp0"
if exist "C:\Espressif\tools\python\pythonw.exe" (
  start "" "C:\Espressif\tools\python\pythonw.exe" "%~dp0app.py"
) else (
  python "%~dp0app.py"
  if errorlevel 1 pause
)
