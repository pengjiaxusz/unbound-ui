<#
.SYNOPSIS
    Layered template sync, batch update, and selective commit pipeline (Template Sync Fast-Track)

.DESCRIPTION
    Automates template sync, audit, and selective commit workflow across single or multiple projects:
    1. Sub-second detection of pengj-templates-cli binary (local target build > PATH > fallback cargo run);
    2. Batch project directories (-Projects @(...));
    3. Default dry run mode: rapid diagnosis of alignment and drift states via audit;
    4. Single-shot update on -Apply with -SyncSkills enabled by default;
    5. Selective staging red-line enforcement: strictly stages only template-affected files and manifest, never git add .;
    6. Automatic selective commit and push on -Apply by default (-NoCommit / -NoPush to opt out), so applied projects never leave template updates uncommitted.

.PARAMETER Projects
    List of target project paths. Supports multiple paths e.g. @("D:\repo1", "D:\repo2"). Defaults to @("."). 

.PARAMETER Apply
    Execute actual update. If omitted, performs read-only pre-flight audit (Dry Run).

.PARAMETER SyncSkills
    Align and overwrite managed skill assets and scripts directly (default $true).

.PARAMETER Commit
    Kept for compatibility: the selective commit is the DEFAULT on -Apply. Use -NoCommit to disable it.

.PARAMETER Push
    Kept for compatibility: the push is the DEFAULT on -Apply. Use -NoPush to disable it.

.PARAMETER NoCommit
    Skip the automatic selective commit after -Apply (also skips push).

.PARAMETER NoPush
    Skip the automatic push but still create the selective commit.

.PARAMETER CommitMessage
    Custom commit message. If omitted, the language follows the target project's commit_zh option (Chinese or English Conventional Commit subject).

.PARAMETER Diff
    Display detailed drift diffs in pre-flight mode.

.EXAMPLE
    # 1. Rapid read-only pre-flight audit of template drifts
    pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("D:\pengj\proj-a", "D:\pengj\proj-b")

    # 2. Batch update, selective commit, and push in a single tool call (default behavior)
    pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("D:\pengj\proj-a", "D:\pengj\proj-b") -Apply

    # 3. Update without committing (explicit opt-out)
    pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("D:\pengj\proj-a") -Apply -NoCommit
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]]$Projects = @("."),

    [switch]$Apply,
    [bool]$SyncSkills = $true,
    [switch]$Commit,
    [switch]$Push,
    [switch]$NoCommit,
    [switch]$NoPush,
    [string]$CommitMessage = "",
    [switch]$Diff
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# -Apply commits and pushes the applied project by default; -NoCommit / -NoPush opt out.
# -Commit / -Push remain accepted for compatibility with existing command lines.
$doCommit = -not $NoCommit.IsPresent
$doPush = $doCommit -and (-not $NoPush.IsPresent)

# 1. Discover or resolve optimal pengj-templates-cli execution path
function Find-CliCommand {
    if ($env:PENGJ_TEMPLATES_CLI -and (Test-Path $env:PENGJ_TEMPLATES_CLI)) {
        return @{ Type = "bin"; Cmd = $env:PENGJ_TEMPLATES_CLI }
    }
    $scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
# Walk up to repository root (detect Cargo.toml)
    $curr = $scriptDir
    for ($i = 0; $i -lt 6; $i++) {
        if (Test-Path (Join-Path $curr "Cargo.toml")) {
            $relBin = Join-Path $curr "target\release\pengj-templates-cli.exe"
            if (Test-Path $relBin) {
                return @{ Type = "bin"; Cmd = $relBin }
            }
            $dbgBin = Join-Path $curr "target\debug\pengj-templates-cli.exe"
            if (Test-Path $dbgBin) {
                return @{ Type = "bin"; Cmd = $dbgBin }
            }
            break
        }
        $parent = Split-Path -Parent $curr
        if (-not $parent -or $parent -eq $curr) { break }
        $curr = $parent
    }
    $pathBin = Get-Command "pengj-templates-cli" -ErrorAction SilentlyContinue
    if ($pathBin) {
        return @{ Type = "bin"; Cmd = $pathBin.Source }
    }
    return @{ Type = "cargo"; Cmd = "cargo" }
}

$cliInfo = Find-CliCommand

function Invoke-Cli {
    param([string[]]$CommandArgs)
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    if ($cliInfo.Type -eq "bin") {
        $psi.FileName = $cliInfo.Cmd
        $psi.Arguments = ($CommandArgs -join " ")
    } else {
        $psi.FileName = "cargo"
        $psi.Arguments = (@("run", "-q", "-p", "pengj-templates-cli", "--") + $CommandArgs) -join " "
    }
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8

    $p = [System.Diagnostics.Process]::Start($psi)
    $stdout = $p.StandardOutput.ReadToEnd()
    $stderr = $p.StandardError.ReadToEnd()
    $p.WaitForExit()

    return [PSCustomObject]@{
        ExitCode = $p.ExitCode
        Output   = $stdout.Trim()
        Error    = $stderr.Trim()
    }
}

function Invoke-Git {
    param(
        [string]$WorkingDir,
        [string[]]$CommandArgs
    )
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = "git"
    $psi.Arguments = ($CommandArgs -join " ")
    $psi.WorkingDirectory = $WorkingDir
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8

    $p = [System.Diagnostics.Process]::Start($psi)
    $stdout = $p.StandardOutput.ReadToEnd()
    $stderr = $p.StandardError.ReadToEnd()
    $p.WaitForExit()

    return [PSCustomObject]@{
        ExitCode = $p.ExitCode
        Output   = $stdout.Trim()
        Error    = $stderr.Trim()
    }
}

function Get-DefaultCommitMessage {
    param([string]$ProjectDir)
    $scope = "template"
    $commitlintPath = Join-Path $ProjectDir "commitlint.config.js"
    if (Test-Path $commitlintPath) {
        $content = Get-Content $commitlintPath -Raw
        if ($content -match "scope-enum" -or $content -match "rules:\s*\{") {
            if ($content -notmatch "['""]template['""]") {
                if ($content -match "['""]ai-skill['""]") {
                    $scope = "ai-skill"
                } else {
                    $scope = ""
                }
            }
        }
    }
    # Commit language axis: follow the applied project's commit_zh, otherwise the
    # commit-msg hook rejects the generated commit subject.
    $commitZh = $true
    $manifestPath = Join-Path $ProjectDir ".pengj-templates.json"
    if (Test-Path $manifestPath) {
        try {
            $manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
            if ($null -ne $manifest.options.commit_zh) {
                $commitZh = [bool]$manifest.options.commit_zh
            }
        } catch {
            Write-Warning "Cannot read commit_zh from manifest, assuming Chinese: $($_.Exception.Message)"
        }
    }

    # Rendered from the commit_zh axis: an EN-commit project never contains CJK text.
    $localizedSubject = '同步分层模板更新'
    if (-not $commitZh) { $localizedSubject = "sync layered template updates" }
    if ($scope) { return "chore($scope): $localizedSubject" }
    return "chore: $localizedSubject"
}

$summaryRows = @()
$hasAnyViolations = $false

$targetProjects = @()
foreach ($p in $Projects) {
    if ($p -match ",") {
        $targetProjects += ($p -split ",") | ForEach-Object { $_.Trim() }
    } else {
        $targetProjects += $p.Trim()
    }
}
$targetProjects = $targetProjects | Where-Object { $_ }
if ($targetProjects.Count -eq 0) {
    $targetProjects = @(".")
}

Write-Host ">>> Using CLI invocation mode: $($cliInfo.Type) ($($cliInfo.Cmd))" -ForegroundColor Cyan

foreach ($rawProj in $targetProjects) {
    if (-not (Test-Path $rawProj)) {
        Write-Warning "Project path does not exist, skipping: $rawProj"
        continue
    }
    $projDir = (Resolve-Path $rawProj).Path
    $manifestPath = Join-Path $projDir ".pengj-templates.json"

    if (-not (Test-Path $manifestPath)) {
        Write-Warning "Project unmanaged (missing .pengj-templates.json), skipping: $projDir"
        continue
    }

    $projectName = Split-Path -Leaf $projDir

    # ---------------- Mode 1: Pre-Flight Audit (Dry Run) ----------------
    if (-not $Apply) {
        Write-Host "`n>>> [Pre-Flight] Auditing project: $projectName ($projDir)" -ForegroundColor Cyan
        $auditArgs = @("audit", "--dir", "`"$projDir`"", "--json")
        $auditRes = Invoke-Cli -CommandArgs $auditArgs
        if ($auditRes.ExitCode -ne 0 -and -not $auditRes.Output) {
            Write-Error "[$projectName] Audit failed: $($auditRes.Error)"
            continue
        }

        try {
            $auditJson = $auditRes.Output | ConvertFrom-Json
        } catch {
            Write-Error "[$projectName] Failed to parse Audit JSON output: $($auditRes.Output)"
            continue
        }

        $inSync = ($auditJson.items | Where-Object { $_.status -eq "InSync" }).Count
        $customized = ($auditJson.items | Where-Object { $_.status -eq "ProjectCustomized" }).Count
        $upstreamNewer = ($auditJson.items | Where-Object { $_.status -eq "UpstreamNewer" }).Count
        $violations = ($auditJson.items | Where-Object { $_.status -in @("ViolatedManagedBlock", "LocallyModifiedFile") }).Count

        if ($auditJson.has_violations) {
            $hasAnyViolations = $true
            Write-Host "  [Warning] Detected $violations violation drifts!" -ForegroundColor Red
        }

        if ($Diff) {
            $diffArgs = @("audit", "--dir", "`"$projDir`"", "--diff")
            $diffRes = Invoke-Cli -CommandArgs $diffArgs
            Write-Host $diffRes.Output
        }

        $summaryRows += [PSCustomObject]@{
            Project       = $projectName
            Mode          = "DryRun"
            InSync        = $inSync
            Customized    = $customized
            UpstreamNewer = $upstreamNewer
            Violations    = $violations
            Action        = if ($upstreamNewer -gt 0) { "Recommend -Apply" } else { "Up to date" }
        }
        continue
    }

    # ---------------- Mode 2: Execute Update (-Apply) ----------------
    Write-Host "`n>>> [Apply] Synchronizing updates: $projectName ($projDir)" -ForegroundColor Cyan
    $updateArgs = @("update", "--dir", "`"$projDir`"")
    if ($SyncSkills) {
        $updateArgs += "--sync-skills"
    }

    $updateRes = Invoke-Cli -CommandArgs $updateArgs
    if ($updateRes.ExitCode -ne 0) {
        Write-Error "[$projectName] Update failed: $($updateRes.Error) $($updateRes.Output)"
        continue
    }

    Write-Host $updateRes.Output

    # Parse files affected by update
    $updatedFiles = @()
    $createdFiles = @()
    $conflictedFiles = @()
    $needsReviewFiles = @()

    foreach ($line in ($updateRes.Output -split "`r?`n")) {
        if ($line -match '^\s*\[(?:\u66f4\u65b0|Updated?)\]\s+(.+)$') {
            $updatedFiles += $matches[1].Trim()
        } elseif ($line -match '^\s*\[(?:\u65b0\u589e|Added?)\]\s+(.+)$') {
            $createdFiles += $matches[1].Trim()
        } elseif ($line -match '^\s*\[(?:\u51b2\u7a81|Conflicted?)\]\s+(.+)$') {
            $conflictedFiles += $matches[1].Trim()
        } elseif ($line -match '^\s*\[(?:\u5f85\u590d\u6838|NeedsReview)\]\s+(.+)$') {
            $needsReviewFiles += $matches[1].Trim()
        }
    }

    $stagedFiles = @()
    # Selective staging red line: only git add files modified by update and manifest
    $candidateFiles = @($updatedFiles + $createdFiles + @(".pengj-templates.json")) | Select-Object -Unique
    foreach ($rel in $candidateFiles) {
        $fullPath = Join-Path $projDir $rel
        if (Test-Path $fullPath) {
            $addRes = Invoke-Git -WorkingDir $projDir -CommandArgs @("add", "`"$rel`"")
            if ($addRes.ExitCode -eq 0) {
                $stagedFiles += $rel
            } else {
                Write-Warning "[$projectName] Staging file failed: $rel - $($addRes.Error)"
            }
        }
    }

    $commitStatus = "Skipped Commit"
    $pushStatus = "Skipped Push"

    if ($doCommit) {
        $cachedDiff = Invoke-Git -WorkingDir $projDir -CommandArgs @("diff", "--cached", "--name-only")
        if ($cachedDiff.Output) {
            $msg = if ($CommitMessage) { $CommitMessage } else { Get-DefaultCommitMessage -ProjectDir $projDir }
            Write-Host "  -> Selectively committing changes: $msg" -ForegroundColor Yellow
            $cRes = Invoke-Git -WorkingDir $projDir -CommandArgs @("commit", "-m", "`"$msg`"")
            if ($cRes.ExitCode -ne 0) {
                Write-Error "[$projectName] git commit failed: $($cRes.Error) $($cRes.Output)"
                $commitStatus = "Commit Failed"
            } else {
                $commitStatus = "Committed"
                if ($doPush) {
                    Write-Host "  -> Pushing to remote repository..." -ForegroundColor Yellow
                    $pRes = Invoke-Git -WorkingDir $projDir -CommandArgs @("push")
                    if ($pRes.ExitCode -ne 0) {
                        Write-Error "[$projectName] git push failed: $($pRes.Error) $($pRes.Output)"
                        $pushStatus = "Push Failed"
                    } else {
                        $pushStatus = "Pushed"
                    }
                }
            }
        } else {
            $commitStatus = "No staged changes"
            $pushStatus = "No changes"
        }
    }

    $summaryRows += [PSCustomObject]@{
        Project      = $projectName
        Mode         = "Apply"
        Updated      = $updatedFiles.Count
        Created      = $createdFiles.Count
        Conflicted   = $conflictedFiles.Count
        NeedsReview  = $needsReviewFiles.Count
        CommitStatus = $commitStatus
        PushStatus   = $pushStatus
    }
}

Write-Host "`n========================= Execution Summary =========================" -ForegroundColor Green
$summaryRows | Format-Table -AutoSize

if ($hasAnyViolations) {
    Write-Host "[Notice] Violation drifts detected; follow standards to resolve or report upstream!`n" -ForegroundColor Yellow
}