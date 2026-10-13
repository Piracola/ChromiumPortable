@echo off
rem ChromiumPortable build launcher. ASCII-only on purpose: this file runs
rem before any codepage switch, so non-ASCII here would print as mojibake.
chcp 65001 >nul
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tui.ps1" %*
