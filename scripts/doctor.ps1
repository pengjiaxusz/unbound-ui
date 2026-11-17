# scripts/doctor.ps1 - project environment check.
#
# Checks only the tools the selected template layers actually use, then runs
# every custom check in scripts/doctor.d/*.ps1 in sorted order.
#
# Usage:
#   pwsh -NoProfile -File scripts/doctor.ps1          report only
#   pwsh -NoProfile -File scripts/doctor.ps1 --fix    also activate the commit hooks
#   (the just recipe forwards `just doctor --fix` unchanged on both platforms)
#
# Custom checks: drop *.ps1 files into scripts/doctor.d/. Each script runs from
# the project root with DOCTOR_ROOT set to the project root and DOCTOR_FIX=1
# when --fix is given. Exit 0 = pass, non-zero = failure; output is shown as-is.

$Fix = $false
foreach ($arg in $args) {
    switch ($arg.ToLowerInvariant()) {
        "--fix" { $Fix = $true }
        "-fix" { $Fix = $true }
        "-h" {
            Write-Host "usage: pwsh -NoProfile -File scripts/doctor.ps1 [--fix]"
            exit 0
        }
        "--help" {
            Write-Host "usage: pwsh -NoProfile -File scripts/doctor.ps1 [--fix]"
            exit 0
        }
        default {
            Write-Host "[doctor] unknown argument: $arg"
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
    Report-Fail "git not found"
}


if (Test-Tool just) {
    Report-Ok "just $((& just --version) -replace '^just ', '')"
} else {
    Report-Fail "just not found; install: https://github.com/casey/just#installation"
}



if (Test-Tool cargo) {
    Report-Ok "cargo $((& cargo --version) -replace '^cargo ', '')"
} else {
    Report-Fail "cargo not found; install a Rust toolchain"
}
if (Test-Tool rustc) {
    Report-Ok "rustc $((& rustc --version) -replace '^rustc ', '')"
} else {
    Report-Fail "rustc not found; install a Rust toolchain"
}



if (Test-Tool bque) {
    Report-Ok "bque $((& bque --version 2>$null | Select-Object -First 1))"
} else {
    Report-Warn "bque not found; install: irm https://github.com/pengjiaxusz/bque/releases/latest/download/install.ps1 | iex"
}



if (Test-Tool pengj-templates-cli) {
    Report-Ok "pengj-templates-cli $((& pengj-templates-cli --version 2>$null | Select-Object -First 1))"
} else {
    Report-Warn "pengj-templates-cli not found; install: irm https://github.com/pengjiaxusz/pengj-templates/releases/latest/download/install.ps1 | iex"
}



if (Test-Tool node) {
    Report-Ok "node $((& node --version 2>$null))"
} else {
    Report-Warn "node not found; it is required by the mcp-bootstrap skill (install Node.js or drop that skill from options.skills)"
}



# --- commit gate activation ------------------------------------------------

$inside = $false
if (Test-Tool git) {
    $inside = [bool](& git rev-parse --show-toplevel 2>$null)
}
if ($inside) {
    $local = (& git config --local --get core.hooksPath 2>$null)
    $globalHooks = (& git config --global --get core.hooksPath 2>$null)
    $legacyGlobal = [bool]($globalHooks -and $globalHooks -match "pengj-templates[\\/]hooks")

    if (-not (Test-Path ".githooks/pre-commit") -or -not (Test-Path ".githooks/commit-msg")) {
        Report-Fail ".githooks is incomplete; expected pre-commit and commit-msg"
    } elseif ($local -eq ".githooks") {
        Report-Ok "commit hooks active (core.hooksPath=.githooks)"
    } elseif ($local) {
        Report-Warn "core.hooksPath is '$local', not .githooks; the commit gate is not active here"
    } elseif ($Fix) {
        # Activate through the first available path, but judge the outcome by
        # the resulting git state: an informational line failing to print must
        # not turn a successful activation into an error.
        if ((Test-Path ".githooks/install.sh") -and (Test-Tool sh)) {
            & sh .githooks/install.sh | Out-Null
        } elseif (Test-Tool pengj-templates-cli) {
            & pengj-templates-cli gate hooks install | Out-Null
        } else {
            & git config core.hooksPath .githooks | Out-Null
        }
        & git update-index --chmod=+x .githooks/pre-commit .githooks/commit-msg 2>$null | Out-Null
        if ((& git config --local --get core.hooksPath 2>$null) -eq ".githooks") {
            Report-Ok "commit hooks activated (core.hooksPath=.githooks)"
        } else {
            Report-Fail "could not activate the commit hooks; run: git config core.hooksPath .githooks"
        }
    } elseif ($legacyGlobal) {
        Report-Warn "commit hooks are active through the legacy global dispatcher; run 'pwsh -NoProfile -File scripts/doctor.ps1 --fix' to switch to per-repository activation"
    } else {
        Report-Warn "commit hooks are not active; run 'pwsh -NoProfile -File scripts/doctor.ps1 -Fix' or 'git config core.hooksPath .githooks'"
    }

    # Older installs wrote a machine-wide dispatcher. It still forwards, but
    # activation is now per project; report it so the leftover can be removed.
    if ($legacyGlobal) {
        Report-Warn "legacy global gate dispatcher detected ($globalHooks); remove it with: git config --global --unset core.hooksPath"
    }
} else {
    Report-Warn "not a git work tree; skipping the commit hook check"
}


# --- custom checks ---------------------------------------------------------

$custom = 0
$customDir = Join-Path $root "scripts/doctor.d"
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
    Write-Host "[doctor] custom: none (add *.ps1 scripts to scripts/doctor.d/ to extend this check)"
}

Write-Host "[doctor] $script:checks checks: $script:errors errors, $script:warnings warnings"
if ($script:errors -gt 0) {
    exit 1
}
exit 0