@echo off
chcp 65001 >nul
title 清除图标缓存
echo.
echo  正在清除 Windows 图标缓存，桌面和任务栏会闪一下，属正常现象...
echo.
taskkill /f /im explorer.exe >nul 2>&1
ping -n 3 127.0.0.1 >nul
del /a /f /q "%localappdata%\IconCache.db" >nul 2>&1
del /a /f /q "%localappdata%\Microsoft\Windows\Explorer\iconcache*" >nul 2>&1
del /a /f /q "%localappdata%\Microsoft\Windows\Explorer\thumbcache*" >nul 2>&1
start "" "%windir%\explorer.exe"
ping -n 4 127.0.0.1 >nul
tasklist /fi "imagename eq explorer.exe" | find /i "explorer" >nul || start "" "%windir%\explorer.exe"
echo.
echo  完成！图标缓存已重建。
echo.
pause
