@echo off
setlocal EnableExtensions
chcp 65001 >nul
rem 由 pengj-templates（common 层）托管。
rem 仓库环境体检的无交互引导脚本：永不自动下载/安装，永不弹 y/n 阻塞输入。

set "PENGJ_BIN="
if defined LOCALAPPDATA set "PENGJ_BIN=%LOCALAPPDATA%\pengj-templates\bin"
if not defined PENGJ_BIN goto :path_done
if not exist "%PENGJ_BIN%" goto :path_done
echo ";%PATH%;" | findstr /I /C:";%PENGJ_BIN%;" >nul 2>&1
if errorlevel 1 set "PATH=%PATH%;%PENGJ_BIN%"
:path_done

set "HAS_JUST=0"
where just >nul 2>&1 && set "HAS_JUST=1"
set "HAS_CLI=0"
where pengj-templates-cli >nul 2>&1 && set "HAS_CLI=1"
set "HAS_PWSH=0"
where pwsh >nul 2>&1 && set "HAS_PWSH=1"
set "HAS_JUSTFILE=0"
if exist "justfile" set "HAS_JUSTFILE=1"

if "%HAS_JUST%"=="1" if "%HAS_CLI%"=="1" if "%HAS_PWSH%"=="1" if "%HAS_JUSTFILE%"=="1" goto :run_just
if "%HAS_CLI%"=="1" goto :run_cli
goto :missing_tools

:run_just
just doctor %*
exit /b %ERRORLEVEL%

:run_cli
pengj-templates-cli doctor --dir . %*
exit /b %ERRORLEVEL%

:missing_tools
set "DISPLAY_BIN=%%LOCALAPPDATA%%\pengj-templates\bin"
if defined PENGJ_BIN set "DISPLAY_BIN=%PENGJ_BIN%"
>&2 echo [doctor] 检测到缺失必要工具（本脚本不会自动下载或修改本机环境）：
>&2 echo.
>&2 echo   - pengj-templates-cli（约 5–10 MB）
>&2 echo     默认安装路径：%DISPLAY_BIN%
>&2 echo     安装命令（PowerShell）：
>&2 echo       irm https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.ps1 ^| iex

if "%HAS_JUST%"=="1" goto :check_pwsh
>&2 echo.
>&2 echo   - just（约 3–5 MB；若直接调用 CLI 可选装）
>&2 echo     常见安装位置：WinGet / Scoop shims 或 %%USERPROFILE%%\.cargo\bin
>&2 echo     安装方式：winget install Casey.Just / scoop install just / https://github.com/casey/just#installation

:check_pwsh
if "%HAS_PWSH%"=="1" goto :footer
>&2 echo.
>&2 echo   - pwsh（PowerShell 7，约 100 MB；Windows 上的 just 配方需要，若直接调用 CLI 可选装）
>&2 echo     安装命令：winget install Microsoft.PowerShell

:footer
>&2 echo.
>&2 echo 安装完成后请重新运行：
>&2 echo   .pengj-templates\scripts\doctor.cmd
>&2 echo   # 或：just doctor（如需修复本地钩子/链接，再显式追加 --fix）
exit /b 1