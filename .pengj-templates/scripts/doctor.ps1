# .pengj-templates/scripts/doctor.ps1 - 项目环境自检.
#
# 只检查所选模板层实际用到的工具，再按文件名顺序运行 .pengj-templates/scripts/doctor.d/*.ps1
# 下的所有自定义检查。
#
# 用法：
#   pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1                  仅输出报告
#   pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 --fix            同时激活提交钩子
#   pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 --install-cli    下载并安装 pengj-templates-cli
#                                                             （唯一会联网的步骤；不会被 --fix 隐含）
#   （just 配方在两个平台上都原样转发 `just doctor --fix`）
#
# 自定义检查：把 *.ps1 放进 .pengj-templates/scripts/doctor.d/。每个脚本从项目根目录运行，
# DOCTOR_ROOT 指向项目根目录，带 --fix 时设置 DOCTOR_FIX=1。
# 退出码 0 为通过，非 0 为失败；输出按原样显示。

$Fix = $false
$InstallCli = $false
foreach ($arg in $args) {
    switch ($arg.ToLowerInvariant()) {
        "--fix" { $Fix = $true }
        "-fix" { $Fix = $true }
        "--install-cli" { $InstallCli = $true }
        "-installcli" { $InstallCli = $true }
        "-h" {
            Write-Host "用法：pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 [--fix] [--install-cli]"
            exit 0
        }
        "--help" {
            Write-Host "用法：pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 [--fix] [--install-cli]"
            exit 0
        }
        default {
            Write-Host "[doctor] 未知参数： $arg"
            exit 2
        }
    }
}

$ErrorActionPreference = "Continue"

$root = (& git rev-parse --show-toplevel 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $root) {
    $root = (Get-Location).Path
}
Set-Location $root
$env:DOCTOR_ROOT = $root
if ($Fix) {
    $env:DOCTOR_FIX = "1"
} else {
    Remove-Item Env:DOCTOR_FIX -ErrorAction SilentlyContinue
}

$script:checks = 0
$script:errors = 0
$script:warnings = 0

function Report-Ok($message) {
    $script:checks++
    Write-Host "[doctor] ok    $message" -ForegroundColor Green
}

function Report-Warn($message) {
    $script:checks++
    $script:warnings++
    Write-Host "[doctor] warn  $message" -ForegroundColor Yellow
}

function Report-Fail($message) {
    $script:checks++
    $script:errors++
    Write-Host "[doctor] error $message" -ForegroundColor Red
}

function Test-Tool($name) {
    return [bool](Get-Command $name -ErrorAction SilentlyContinue)
}

# --- built-in checks: only what the selected layers use ---------------------

if (Test-Tool git) {
    Report-Ok "git $((& git --version) -replace '^git version ', '')"
} else {
    Report-Fail "未找到 git"
}


if (Test-Tool just) {
    Report-Ok "just $((& just --version) -replace '^just ', '')"
} else {
    Report-Fail "未找到 just；安装地址：https://github.com/casey/just#installation"
}



if (Test-Tool cargo) {
    Report-Ok "cargo $((& cargo --version) -replace '^cargo ', '')"
} else {
    Report-Fail "未找到 cargo；请安装 Rust 工具链"
}
if (Test-Tool rustc) {
    Report-Ok "rustc $((& rustc --version) -replace '^rustc ', '')"
} else {
    Report-Fail "未找到 rustc；请安装 Rust 工具链"
}



if (Test-Tool bque) {
    Report-Ok "bque $((& bque --version 2>$null | Select-Object -First 1))"
} else {
    Report-Warn "未找到 bque；安装命令：irm https://github.com/pengjiaxusz/bque/releases/latest/download/install.ps1 | iex"
}



# --- pengj-templates-cli ---------------------------------------------------
# 提交门禁要它跑检查，agent 层的工作区链接由它的 `repo init` 创建，所以两层都
# 缺不了它：缺失是错误而不是警告。安装是唯一会联网的动作，只由 --install-cli
# 显式触发——`--fix` 永远不联网。
$CliReleaseBase = "https://github.com/pengjiaxusz/pengj-templates/releases/latest/download"
$CliHint = "irm $CliReleaseBase/install.ps1 | iex"

function Install-CliTool {
# 装到独立进程里跑：安装器自己会 exit，不能把 doctor 一起带走。
    $installer = Join-Path ([System.IO.Path]::GetTempPath()) "pengj-install-$PID.ps1"
    try {
        Invoke-RestMethod -Uri "$CliReleaseBase/install.ps1" -OutFile $installer -ErrorAction Stop
    } catch {
        Remove-Item $installer -ErrorAction SilentlyContinue
        return $false
    }
    & pwsh -NoProfile -File $installer
    $code = $LASTEXITCODE
    Remove-Item $installer -ErrorAction SilentlyContinue
    return ($code -eq 0)
}

if (Test-Tool pengj-templates-cli) {
    Report-Ok "pengj-templates-cli $((& pengj-templates-cli --version 2>$null | Select-Object -First 1))"
} elseif ($InstallCli) {
    if (Install-CliTool) {
        if (Test-Tool pengj-templates-cli) {
            Report-Ok "pengj-templates-cli $((& pengj-templates-cli --version 2>$null | Select-Object -First 1))"
        } else {
            Report-Warn "pengj-templates-cli 已安装，但当前终端的 PATH 还没刷新；重开终端后再跑一次 doctor"
        }
    } else {
        Report-Fail "无法自动安装 pengj-templates-cli；请手动安装：$CliHint"
    }
} else {
    Report-Fail "未找到 pengj-templates-cli；提交门禁与 agent 工作区链接都依赖它。安装：$CliHint（或给本脚本加 --install-cli）"
}



if (Test-Tool node) {
    Report-Ok "node $((& node --version 2>$null))"
} else {
    Report-Warn "未找到 node；mcp-bootstrap 技能需要它（请安装 Node.js，或从 options.skills 中移除该技能）"
}



# --- commit gate activation ------------------------------------------------

$inside = $false
if (Test-Tool git) {
    $inside = [bool](& git rev-parse --show-toplevel 2>$null)
}
if ($inside) {
    $local = (& git config --local --get core.hooksPath 2>$null)

    if (-not (Test-Path ".githooks/pre-commit") -or -not (Test-Path ".githooks/commit-msg")) {
        Report-Fail ".githooks 不完整；应包含 pre-commit 与 commit-msg"
    } elseif ($local -eq ".githooks") {
        Report-Ok "提交钩子已激活（core.hooksPath=.githooks）"
    } elseif ($local) {
        Report-Warn "core.hooksPath 为 '$local'，并非 .githooks；提交门禁未在此生效"
    } elseif ($Fix) {
        # Activate through the zero-dependency installer shipped by the gate
        # layer, but judge the outcome by the resulting git state: an
        # informational line failing to print must not turn a successful
        # activation into an error.
        if ((Test-Path ".githooks/install.sh") -and (Test-Tool sh)) {
            & sh .githooks/install.sh | Out-Null
        }
        & git update-index --chmod=+x .githooks/pre-commit .githooks/commit-msg 2>$null | Out-Null
        if ((& git config --local --get core.hooksPath 2>$null) -eq ".githooks") {
            Report-Ok "提交钩子已激活（core.hooksPath=.githooks）"
        } else {
            Report-Fail "无法激活提交钩子；请运行：git config core.hooksPath .githooks"
        }
    } else {
        Report-Warn "提交钩子未激活；请运行 'pwsh -NoProfile -File .pengj-templates/scripts/doctor.ps1 --fix' 或 'git config core.hooksPath .githooks'"
    }
} else {
    Report-Warn "当前不是 git 工作树；跳过提交钩子检查"
}



# --- agent client workspace links -----------------------------------------

if (Test-Path ".agents") {
    if (Test-Tool pengj-templates-cli) {
        & pengj-templates-cli repo status --dir . 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) {
            Report-Ok "工作区链接已就绪（agent 客户端）"
        } elseif ($Fix) {
            & pengj-templates-cli repo init --dir . 2>$null | Out-Null
            $initCode = $LASTEXITCODE
            & pengj-templates-cli repo status --dir . 2>$null | Out-Null
            if ($initCode -eq 0 -and $LASTEXITCODE -eq 0) {
                Report-Ok "工作区链接已创建（agent 客户端）"
            } else {
                Report-Fail "无法创建工作区链接；请运行：pengj-templates-cli repo init"
            }
        } else {
            Report-Fail "工作区链接未就绪；请运行 'just doctor --fix'"
        }
    } else {
        Report-Warn "未找到 pengj-templates-cli；没有它无法检查或创建工作区链接"
    }
}


# --- custom checks ---------------------------------------------------------

$custom = 0
$customDir = Join-Path $root ".pengj-templates/scripts/doctor.d"
if (Test-Path $customDir) {
    Get-ChildItem -Path $customDir -Filter "*.ps1" | Sort-Object Name | ForEach-Object {
        $custom++
        $name = $_.Name
        & pwsh -NoProfile -File $_.FullName
        if ($LASTEXITCODE -eq 0) {
            $script:checks++
            Write-Host "[doctor] custom ok    $name" -ForegroundColor Green
        } else {
            $script:checks++
            $script:errors++
            Write-Host "[doctor] custom error $name" -ForegroundColor Red
        }
    }
}
if ($custom -eq 0) {
    Write-Host "[doctor] 自定义检查：无（在 .pengj-templates/scripts/doctor.d/ 下添加 *.ps1 脚本即可扩展本检查）"
}

Write-Host "[doctor] 共 $script:checks 项检查：$script:errors 个错误，$script:warnings 个警告"
if ($script:errors -gt 0) {
    exit 1
}
exit 0