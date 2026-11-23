@echo off
setlocal EnableExtensions
chcp 65001 >nul
rem 由 pengj-templates（common 层）托管。
rem 仓库一键装机的零前置薄引导脚本：若缺少 pengj-templates-cli 则自动安装，随后转交 `pengj-templates-cli setup --dir .`。

set "PENGJ_BIN="
if defined LOCALAPPDATA set "PENGJ_BIN=%LOCALAPPDATA%\pengj-templates\bin"
if not defined PENGJ_BIN goto :path_done
echo ";%PATH%;" | findstr /I /C:";%PENGJ_BIN%;" >nul 2>&1
if errorlevel 1 set "PATH=%PATH%;%PENGJ_BIN%"
:path_done

where pengj-templates-cli >nul 2>&1
if not errorlevel 1 goto :run_cli

set "DRY_RUN=0"
for %%A in (%*) do (
  if /I "%%~A"=="--dry-run" set "DRY_RUN=1"
)
if "%DRY_RUN%"=="1" goto :dry_run

set "DISPLAY_BIN=%%LOCALAPPDATA%%\pengj-templates\bin"
if defined PENGJ_BIN set "DISPLAY_BIN=%PENGJ_BIN%"
>&2 echo [setup] 正在安装 pengj-templates-cli 到 %DISPLAY_BIN%...
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.ps1 | iex"
if errorlevel 1 exit /b %ERRORLEVEL%

if defined GITHUB_PATH if defined PENGJ_BIN (
  >>"%GITHUB_PATH%" echo %PENGJ_BIN%
)

where pengj-templates-cli >nul 2>&1
if errorlevel 1 (
  >&2 echo [setup] 错误：安装完成后仍未在 PATH 中找到 pengj-templates-cli（预期目录：%DISPLAY_BIN%）。
  exit /b 1
)
goto :run_cli

:dry_run
set "DISPLAY_BIN=%%LOCALAPPDATA%%\pengj-templates\bin"
if defined PENGJ_BIN set "DISPLAY_BIN=%PENGJ_BIN%"
echo [setup] [dry-run] 未检测到 pengj-templates-cli。
echo [setup] [dry-run] 将下载安装 pengj-templates-cli（目标目录：%DISPLAY_BIN%）并执行 pengj-templates-cli setup --dir .
exit /b 0

:run_cli
pengj-templates-cli setup --dir . %*
exit /b %ERRORLEVEL%