@echo off
rem 把 workspace 根的应急解压组件同步进 app 打包资源（7z.exe/7z.dll/UnRAR.exe/许可证）。
rem 升级 7-Zip / WinRAR 内置组件后重跑本脚本再打包即可。
xcopy /Y /I /E "%~dp0..\assets\bundled" "%~dp0src-tauri\resources\bundled"
