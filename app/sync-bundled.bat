@echo off
rem Copy bundled extractors and license files into the app package resources.
rem Run this after updating the bundled 7-Zip or UnRAR files.
xcopy /Y /I /E "%~dp0..\assets\bundled" "%~dp0src-tauri\resources\bundled"
