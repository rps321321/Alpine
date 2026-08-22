@echo off
setlocal

set "ALPINE_CARGO=cargo"
where.exe cargo >NUL 2>NUL
if errorlevel 1 (
  where.exe winget.exe >NUL 2>NUL
  if errorlevel 1 (
    echo Project Alpine source setup requires Cargo or winget. 1>&2
    exit /b 1
  )
  winget.exe install --id Rustlang.Rustup --exact --accept-package-agreements --accept-source-agreements --disable-interactivity
  if errorlevel 1 exit /b 1
  set "ALPINE_CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
  if not exist "%ALPINE_CARGO%" (
    echo Rustup completed but Cargo is unavailable at "%ALPINE_CARGO%". 1>&2
    exit /b 1
  )
)

"%ALPINE_CARGO%" run --locked --release --bin alpine -- setup --repository-root "%~dp0." %*
set "ALPINE_EXIT=%ERRORLEVEL%"
exit /b %ALPINE_EXIT%
