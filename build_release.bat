@echo off
rem Builds the release exe and packs a folder + zip you can share.
cd /d "%~dp0"
cargo build --release
if errorlevel 1 goto :fail

set "DIST=%~dp0dist\cmap-dashboard"
if exist "%~dp0dist" rmdir /s /q "%~dp0dist"
mkdir "%DIST%\data"
copy /y "%~dp0target\release\cmap-dashboard.exe" "%DIST%\" >nul
copy /y "%~dp0data\role.csv" "%DIST%\data\" >nul
copy /y "%~dp0data\project.csv" "%DIST%\data\" >nul
copy /y "%~dp0SHARING.txt" "%DIST%\READ ME.txt" >nul
powershell -NoProfile -Command "Compress-Archive -Path '%DIST%' -DestinationPath '%~dp0dist\cmap-dashboard.zip' -Force"

echo.
echo Done. Share: %~dp0dist\cmap-dashboard.zip
echo (Contains no login details - each person is asked for their own on first run.)
pause
exit /b 0
:fail
echo Build failed.
pause
exit /b 1
