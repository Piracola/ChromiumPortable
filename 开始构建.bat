@echo off
setlocal
cd /d "%~dp0"
set PYTHONUTF8=1
set PYTHONIOENCODING=utf-8

set "PY="
where py >nul 2>nul
if not errorlevel 1 set "PY=py -3"
if not defined PY (
  where python >nul 2>nul
  if not errorlevel 1 set "PY=python"
)
if not defined PY (
  echo [ERROR] Python 3 not found. Install Python 3 and enable "Add to PATH".
  pause
  exit /b 1
)

%PY% scripts\wizard.py
set ERR=%ERRORLEVEL%
echo.
if not "%ERR%"=="0" (
  echo [ERROR] Wizard exited with code %ERR%.
) else (
  echo [OK] Done.
)
pause
exit /b %ERR%
