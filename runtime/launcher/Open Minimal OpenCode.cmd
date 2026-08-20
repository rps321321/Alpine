@echo off
setlocal
set "install_root=%~dp0"
if not exist "%install_root%alpine.exe" set "install_root=%~dp0..\"
"%install_root%alpine.exe" opencode --install-root "%install_root%" --project "%CD%" --allow-legacy-identity %*
set "launcher_exit=%ERRORLEVEL%"
if not "%launcher_exit%"=="0" pause
exit /b %launcher_exit%
