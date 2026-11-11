<#
.SYNOPSIS
    Universal sub-repository / submodule unapplied commit comparison and impact clustering tool.

.DESCRIPTION
    Compares current subrepo/submodule pinned commit against target latest commit (supporting sibling repos, env vars, or remote branches),
    extracts oneline commits between them, and performs multi-dimensional clustering by breaking changes, conventional commits (feat/fix/perf/refactor),
    and domain subsystems (motion, tokens, styles, components).
    Supports fetching (-Fetch) and one-click checkout update (-Update).

.PARAMETER SubrepoPath
    Relative path to sub-repository. Automatically detects common submodule paths if omitted.

.PARAMETER TargetPath
    Target repository path. If omitted, checks $env:<NAME>_DIR, sibling ..\<name>, or subrepo remote.
    Defaults to sub-repository remote branch.

.PARAMETER TargetRef
    Target branch or ref name (defaults to "origin/main", fallback to "origin/master" or "HEAD").

.PARAMETER Fetch
    Fetch latest commits before comparing (default $true).

.PARAMETER Update
    Check out target commit in subrepo after comparing (default $false).

.PARAMETER ConfigPath
    Optional JSON configuration file for custom keyword clustering rules.

.EXAMPLE
    pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "submodules/cha-set"
    pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "submodules/cha-set" -Update
#>

[CmdletBinding()]
param(
    [string]$SubrepoPath = "",
    [string]$TargetPath = "",
    [string]$TargetRef = "",
    [switch]$Fetch = $true,
    [switch]$Update = $false,
    [string]$ConfigPath = ""
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# 1. Auto-detect or validate sub-repository path
function Resolve-SubrepoPath {
    param([string]$Path)
    if ($Path -and (Test-Path $Path)) {
        return (Resolve-Path $Path).Path
    }

# Detect .gitmodules
    if (Test-Path ".gitmodules") {
        $modulePaths = git config --file .gitmodules --get-regexp path | ForEach-Object { ($_ -split ' ')[1] }
        foreach ($mp in $modulePaths) {
            if ($mp -and (Test-Path $mp)) {
                return (Resolve-Path $mp).Path
            }
        }
    }

# Common candidate paths
    $candidates = @("submodules", "external", "vendor", "deps", "packages")
    foreach ($cand in $candidates) {
        if (Test-Path $cand) {
            $subdirs = Get-ChildItem -Path $cand -Directory
            if ($subdirs.Count -eq 1) {
                return $subdirs[0].FullName
            }
        }
    }

    return $null
}

$resolvedSubrepo = Resolve-SubrepoPath $SubrepoPath
if (-not $resolvedSubrepo -or -not (Test-Path $resolvedSubrepo)) {
    Write-Error "Could not locate sub-repository path. Please specify directory via -SubrepoPath, e.g. -SubrepoPath 'submodules/my-lib'"
}

$subrepoName = Split-Path $resolvedSubrepo -Leaf
Write-Host "Target sub-repository detected: $subrepoName ($resolvedSubrepo)" -ForegroundColor DarkGray

# 2. Read currently pinned commit
$oldCommitFull = (git -C $resolvedSubrepo rev-parse HEAD).Trim()
$oldCommitShort = (git -C $resolvedSubrepo rev-parse --short HEAD).Trim()
$oldCommitSubject = (git -C $resolvedSubrepo log -1 --pretty=format:"%s" HEAD).Trim()

# 3. Resolve target commit source
$targetRepoPath = ""
$targetRefResolved = ""

$envVarName = "$($subrepoName.ToUpper().Replace('-', '_'))_DIR"
$envPath = [Environment]::GetEnvironmentVariable($envVarName)

if ($TargetPath -and (Test-Path $TargetPath)) {
    $targetRepoPath = (Resolve-Path $TargetPath).Path
    $targetRefResolved = if ($TargetRef) { $TargetRef } else { "HEAD" }
}
elseif ($envPath -and (Test-Path $envPath)) {
    $targetRepoPath = (Resolve-Path $envPath).Path
    $targetRefResolved = if ($TargetRef) { $TargetRef } else { "HEAD" }
}
elseif (Test-Path "..\\$subrepoName\\.git") {
    $targetRepoPath = (Resolve-Path "..\\$subrepoName").Path
    $targetRefResolved = if ($TargetRef) { $TargetRef } else { "HEAD" }
}
else {
    $targetRepoPath = $resolvedSubrepo
    if ($TargetRef) {
        $targetRefResolved = $TargetRef
    } else {
# Probe default remote branch
        $hasOriginMain = git -C $resolvedSubrepo branch -r --list "origin/main"
        if ($hasOriginMain) {
            $targetRefResolved = "origin/main"
        } else {
            $targetRefResolved = "origin/master"
        }
    }
}

# 4. Execute Fetch
if ($Fetch) {
    Write-Host "Fetching latest commits from remote..." -ForegroundColor DarkGray
    try {
        if (Test-Path "$resolvedSubrepo\.git") {
            try { git -C $resolvedSubrepo fetch origin --quiet } catch {}
        }
        if ($targetRepoPath -ne $resolvedSubrepo -and (Test-Path "$targetRepoPath\.git")) {
            try { git -C $targetRepoPath fetch --quiet } catch {}
        }
    }
    catch {
        Write-Warning "Fetch failed; comparing against existing local cache: $_"
    }
}

$newCommitFull = (git -C $targetRepoPath rev-parse $targetRefResolved).Trim()
$newCommitShort = (git -C $targetRepoPath rev-parse --short $targetRefResolved).Trim()
$newCommitSubject = (git -C $targetRepoPath log -1 --pretty=format:"%s" $targetRefResolved).Trim()

Write-Host ""
Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host "             $subrepoName Subrepo Unapplied Commits Analysis               " -ForegroundColor Cyan
Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host "[Current Pin]: $oldCommitShort ($oldCommitFull)" -ForegroundColor Yellow
Write-Host "               └─ $oldCommitSubject" -ForegroundColor DarkGray
Write-Host "[Target Pin]:  $newCommitShort ($newCommitFull) [source: $targetRepoPath -> $targetRefResolved]" -ForegroundColor Green
Write-Host "               └─ $newCommitSubject" -ForegroundColor DarkGray
Write-Host "-----------------------------------------------------------------" -ForegroundColor Gray

if ($oldCommitFull -eq $newCommitFull) {
    Write-Host "🎉 Current repository is already aligned with $subrepoName target pin; no update needed!" -ForegroundColor Green
    return
}

# 5. Extract oneline diff list
$commitsRaw = @()
try {
    $commitsRaw = git -C $targetRepoPath log "$oldCommitFull..$newCommitFull" --oneline --no-merges
}
catch {
    try {
        $commitsRaw = git -C $resolvedSubrepo log "$oldCommitFull..$newCommitFull" --oneline --no-merges
    }
    catch {
    Write-Error "Comparison failed: unable to resolve commit range $oldCommitShort..$newCommitShort in repository"
    }
}

$commitCount = $commitsRaw.Count
Write-Host "Unapplied commits found: $commitCount" -ForegroundColor Cyan
Write-Host ""

# 6. Multi-dimensional impact clustering
$breakingCommits = @()
$featCommits = @()
$fixCommits = @()
$perfCommits = @()
$refactorCommits = @()
$motionCommits = @()
$tokenCommits = @()
$styleCommits = @()
$componentCommits = @()
$otherCommits = @()

foreach ($line in $commitsRaw) {
    if (-not $line) { continue }
    $trimmed = $line.Trim()

    # 1. Prioritize Breaking Changes
    if ($trimmed -match "(^[a-f0-9]+\s+[a-z0-9\-]+!\:|BREAKING CHANGE)") {
        $breakingCommits += $trimmed
    }

    # 2. Domain keyword clustering
    if ($trimmed -match "(motion|easing|animate|transition)") {
        $motionCommits += $trimmed
    }
    elseif ($trimmed -match "(token|theme|palette|color)") {
        $tokenCommits += $trimmed
    }
    elseif ($trimmed -match "(styles|cascade-layer|layer|subpath|css)") {
        $styleCommits += $trimmed
    }
    elseif ($trimmed -match "(component|widget|control)") {
        $componentCommits += $trimmed
    }

    # 3. Conventional Commits classification
    if ($trimmed -match "^[a-f0-9]+\s+feat(\([^\)]+\))?\:") {
        $featCommits += $trimmed
    }
    elseif ($trimmed -match "^[a-f0-9]+\s+fix(\([^\)]+\))?\:") {
        $fixCommits += $trimmed
    }
    elseif ($trimmed -match "^[a-f0-9]+\s+perf(\([^\)]+\))?\:") {
        $perfCommits += $trimmed
    }
    elseif ($trimmed -match "^[a-f0-9]+\s+refactor(\([^\)]+\))?\:") {
        $refactorCommits += $trimmed
    }
    elseif ($trimmed -notmatch "(motion|token|styles|component|feat|fix|perf|refactor)") {
        $otherCommits += $trimmed
    }
}

Write-Host "[Subsystems & Clustered Impact]" -ForegroundColor Yellow

if ($breakingCommits.Count -gt 0) {
    Write-Host "  💥 Breaking Changes [$($breakingCommits.Count) commits] - Needs critical review:" -ForegroundColor Red
    $breakingCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($motionCommits.Count -gt 0) {
    Write-Host "  🎬 Motion & Transition System (Motion) [$($motionCommits.Count) commits]:" -ForegroundColor Magenta
    $motionCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($tokenCommits.Count -gt 0) {
    Write-Host "  🎨 Design Tokens & Color System (Tokens & Themes) [$($tokenCommits.Count) commits]:" -ForegroundColor Yellow
    $tokenCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($styleCommits.Count -gt 0) {
    Write-Host "  📐 Styles & Layout Packaging (Styles & Layout) [$($styleCommits.Count) commits]:" -ForegroundColor Green
    $styleCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($componentCommits.Count -gt 0) {
    Write-Host "  🧩 Standard Widgets & Component API Changes (Components) [$($componentCommits.Count) commits]:" -ForegroundColor Cyan
    $componentCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($featCommits.Count -gt 0 -and $componentCommits.Count -eq 0) {
    Write-Host "  ✨ Features & Capabilities (Features) [$($featCommits.Count) commits]:" -ForegroundColor Cyan
    $featCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($fixCommits.Count -gt 0) {
    Write-Host "  🐛 Bug Fixes & Stability (Bug Fixes) [$($fixCommits.Count) commits]:" -ForegroundColor DarkYellow
    $fixCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($perfCommits.Count -gt 0) {
    Write-Host "  ⚡ Performance Optimizations (Performance) [$($perfCommits.Count) commits]:" -ForegroundColor Green
    $perfCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

if ($otherCommits.Count -gt 0) {
    Write-Host "  🛠️ Governance & Chore [$($otherCommits.Count) commits]:" -ForegroundColor DarkGray
    $otherCommits | ForEach-Object { Write-Host "     - $_" }
    Write-Host ""
}

Write-Host "-----------------------------------------------------------------" -ForegroundColor Gray
Write-Host "[Full Unapplied Commits List (git log oneline)]:" -ForegroundColor Yellow
$commitsRaw | ForEach-Object { Write-Host "  $_" }
Write-Host "=================================================================" -ForegroundColor Cyan
Write-Host ""

# 7. Perform checkout update (-Update)
if ($Update) {
    # Check workspace cleanliness
    $subrepoStatus = (git -C $resolvedSubrepo status --short)
    if ($subrepoStatus) {
        Write-Warning "Sub-repository workspace has uncommitted changes; aborting checkout. Stash or commit subrepo changes first."
        return
    }

    Write-Host "Checking out sub-repository $subrepoName to target commit: $newCommitShort ..." -ForegroundColor Yellow
    git -C $resolvedSubrepo checkout $newCommitFull
    $updatedCommit = (git -C $resolvedSubrepo rev-parse --short HEAD).Trim()
        Write-Host "✅ Sub-repository successfully updated to: $updatedCommit" -ForegroundColor Green
    Write-Host ""
        Write-Host "Recommended Next Steps:" -ForegroundColor Cyan
        Write-Host "  1. Run host compilation and type checks (e.g. cargo check, pnpm build, just build)"
        Write-Host "  2. Adapt affected business components and design tokens based on clustering report"
        Write-Host "  3. Run automated tests and regression verification gates"
        Write-Host "  4. Create a conventional commit, e.g.: chore(deps): update $subrepoName to $newCommitShort and adapt callers"
}
else {
    Write-Host "💡 Hint: To check out and update this subrepo to the target pin directly, run:" -ForegroundColor DarkYellow
    Write-Host "  pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath '$SubrepoPath' -Update" -ForegroundColor White
}