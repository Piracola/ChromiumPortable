@echo off
rem 双击或命令行都行，参数原样转给 build.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
pause
