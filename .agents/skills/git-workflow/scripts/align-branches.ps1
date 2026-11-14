<#
.SYNOPSIS
    Batch branch and worktree alignment and safe push tool (Align Branches)

.DESCRIPTION
    Batch resets and aligns specified or all local branches and associated worktrees to target commit (defaults to current integration branch tip),
    safely pushing to remote using --force-with-lease.
    Branches checked out in worktrees are reset via git reset --hard in their worktree directory; other branches are updated via git branch -f.

.PARAMETER TargetCommit
    Target commit or branch name to align to. Defaults to active integration branch.

.PARAMETER Branches
    List of branch names to align. If omitted, aligns all local branches except target branch.

.PARAMETER Apply
    Execute actual reset and push. Dry run by default.

.PARAMETER NoPush
    Skip remote push.
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$TargetCommit = "",

    [Parameter(Position = 1)]
    [string[]]$Branches = @(),

    [switch]$Apply,
    [switch]$NoPush,
    [switch]$Force
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Format-GitArg {
    param([string]$Arg)
    if ($Arg -match '[\s"]') {
        return '"' + ($Arg -replace '"', '\"') + '"'
    }
    return $Arg
}

function Invoke-Git {
    param(
        [string[]]$CommandArgs,
        [string]$WorkingDir = "",
        [int]$TimeoutSeconds = 300
    )
    $pinfo = New-Object System.Diagnostics.ProcessStartInfo
    $pinfo.FileName = "git"
    $pinfo.Arguments = ($CommandArgs | ForEach-Object { Format-GitArg $_ }) -join " "
    $pinfo.RedirectStandardOutput = $true
    $pinfo.RedirectStandardError = $true
    $pinfo.UseShellExecute = $false
    $pinfo.CreateNoWindow = $true
    $pinfo.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $pinfo.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    if ($WorkingDir) {
        $pinfo.WorkingDirectory = $WorkingDir
    }

    # Non-interactive guard: disable interactive credential prompts and external editor popups
    $isWin = [System.Environment]::OSVersion.Platform -like "*Win*" -or $IsWindows
    $pinfo.Environment["GIT_TERMINAL_PROMPT"] = "0"
    $pinfo.Environment["GIT_OPTIONAL_LOCKS"] = "0"
    if ($isWin) {
        $pinfo.Environment["GIT_EDITOR"] = "cmd.exe /c exit 0"
        $pinfo.Environment["GIT_SEQUENCE_EDITOR"] = $pinfo.Environment["GIT_EDITOR"]
        $pinfo.Environment["GIT_PAGER"] = "cat"
    } else {
        $pinfo.Environment["GIT_EDITOR"] = "true"
        $pinfo.Environment["GIT_SEQUENCE_EDITOR"] = $pinfo.Environment["GIT_EDITOR"]
        $pinfo.Environment["GIT_PAGER"] = "cat"
    }

    $process = [System.Diagnostics.Process]::Start($pinfo)
    $outTask = $process.StandardOutput.ReadToEndAsync()
    $errTask = $process.StandardError.ReadToEndAsync()

    $timeoutMs = if ($TimeoutSeconds -gt 0) { $TimeoutSeconds * 1000 } else { [System.Threading.Timeout]::Infinite }
    $exited = $process.WaitForExit($timeoutMs)

    if (-not $exited) {
        try {
            $process.Kill($true)
        } catch {
            try { $process.Kill() } catch {}
        }
        $cmdStr = "git " + (($CommandArgs | ForEach-Object { Format-GitArg $_ }) -join " ")
        throw "Git command timed out (exceeded $($TimeoutSeconds)s) and was terminated: $cmdStr"
    }

    $process.WaitForExit()
    [System.Threading.Tasks.Task]::WaitAll(@($outTask, $errTask))

    return [PSCustomObject]@{
        ExitCode = $process.ExitCode
        Output   = $outTask.Result.Trim()
        Error    = $errTask.Result.Trim()
    }
}

function Get-RemoteNames {
    $remotes = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    $res = Invoke-Git @("remote")
    if ($res.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($res.Output))) {
        foreach ($line in ($res.Output -split "`r?`n")) {
            $r = $line.Trim()
            if (-not [string]::IsNullOrWhiteSpace($r)) {
                $remotes.Add($r) | Out-Null
            }
        }
    }
    return $remotes
}

function Test-ValidBranchName {
    param(
        [string]$BranchName,
        [System.Collections.Generic.HashSet[string]]$RemoteNames = $null
    )
    if ([string]::IsNullOrWhiteSpace($BranchName)) { return $false }
    $b = $BranchName.Trim()

    # 1. Never equal to any remote name (e.g. origin, upstream)
    if ($RemoteNames -and $RemoteNames.Contains($b)) { return $false }

    # 2. Never contain ref namespaces or recursive prefixes (heads/..., refs/..., remotes/...)
    if ($b.StartsWith("heads/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("refs/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("remotes/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    # 3. Never contain internal backup namespaces
    if ($b.StartsWith("sync-backup/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("cleanup-backup/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    # 4. Never contain nested path fragments (/heads/, /refs/, /remotes/)
    if ($b -match "(?i)(?:^|/)(?:refs|heads|remotes)(?:/|$)") {
        return $false
    }

    # 5. Never start with remote/ (e.g. origin/main)
    if ($RemoteNames) {
        foreach ($r in $RemoteNames) {
            if ($b.StartsWith("$r/", [System.StringComparison]::OrdinalIgnoreCase)) {
                return $false
            }
        }
    }

    # 6. Special reserved names
    if ($b -eq "HEAD" -or $b.StartsWith("HEAD/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    # 7. Git check-ref-format validation
    $checkRes = Invoke-Git @("check-ref-format", "--branch", $b)
    if ($checkRes.ExitCode -ne 0) {
        return $false
    }

    return $true
}

function Get-CleanBranchName {
    param(
        [string]$RefOrBranch,
        [System.Collections.Generic.HashSet[string]]$RemoteNames = $null
    )
    if ([string]::IsNullOrWhiteSpace($RefOrBranch)) { return "" }
    $clean = $RefOrBranch.Trim()

    # Strip prefixes layer by layer
    while ($clean.StartsWith("refs/heads/", [System.StringComparison]::OrdinalIgnoreCase) -or
           $clean.StartsWith("refs/remotes/", [System.StringComparison]::OrdinalIgnoreCase)) {
        if ($clean.StartsWith("refs/heads/", [System.StringComparison]::OrdinalIgnoreCase)) {
            $clean = $clean.Substring(11).Trim()
        } elseif ($clean.StartsWith("refs/remotes/", [System.StringComparison]::OrdinalIgnoreCase)) {
            $clean = $clean.Substring(13).Trim()
        }
    }

    if ($RemoteNames) {
        foreach ($r in $RemoteNames) {
            if ($clean.StartsWith("$r/", [System.StringComparison]::OrdinalIgnoreCase)) {
                $clean = $clean.Substring($r.Length + 1).Trim()
            }
        }
    }

    if (Test-ValidBranchName $clean $RemoteNames) {
        return $clean
    }
    return ""
}

function Resolve-IntegrationBranch {
    param(
        [string]$ExplicitBranch,
        [string]$DeclaredBranch,
        [bool]$HasRemote,
        [System.Collections.Generic.HashSet[string]]$RemoteNames = $null
    )

    if (-not [string]::IsNullOrWhiteSpace($ExplicitBranch)) {
        $cExplicit = Get-CleanBranchName $ExplicitBranch $RemoteNames
        if (-not [string]::IsNullOrWhiteSpace($cExplicit)) {
            return $cExplicit
        }
        throw "Explicitly specified integration branch name is invalid: '$ExplicitBranch'"
    }

    $symRef = Invoke-Git @("symbolic-ref", "-q", "HEAD")
    if ($symRef.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($symRef.Output))) {
        $trimmed = $symRef.Output.Trim()
        if ($trimmed.StartsWith("refs/heads/")) {
            $cand = Get-CleanBranchName ($trimmed.Substring(11)) $RemoteNames
            if (-not [string]::IsNullOrWhiteSpace($cand)) {
                Write-Host "💡 Automatically selected active branch in current directory as integration branch: '$cand'" -ForegroundColor Cyan
                return $cand
            }
        }
    }

    $currentPath = (Get-Location).Path
    try { $currentPath = (Resolve-Path $currentPath).Path } catch {}
    $wtList = Invoke-Git @("worktree", "list", "--porcelain")
    if ($wtList.ExitCode -eq 0) {
        $lines = $wtList.Output -split "`r?`n"
        $wPath = ""
        foreach ($line in $lines) {
            if ($line.StartsWith("worktree ")) {
                $wPath = $line.Substring(9).Trim()
                try { $wPath = (Resolve-Path $wPath).Path } catch {}
            } elseif ($line.StartsWith("branch refs/heads/")) {
                $cand = Get-CleanBranchName ($line.Substring(18)) $RemoteNames
                if ($wPath -eq $currentPath -and (-not [string]::IsNullOrWhiteSpace($cand))) {
                    Write-Host "💡 Detected branch checked out in current Worktree as integration branch: '$cand'" -ForegroundColor Cyan
                    return $cand
                }
            }
        }
    }

    $pointsAt = Invoke-Git @("branch", "--points-at", "HEAD", "--format=%(refname)")
    if ($pointsAt.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($pointsAt.Output))) {
        $candidates = [System.Collections.Generic.List[string]]::new()
        foreach ($pLine in ($pointsAt.Output -split "`r?`n")) {
            $pClean = $pLine.Trim()
            if ($pClean.StartsWith("refs/heads/")) {
                $c = Get-CleanBranchName ($pClean.Substring(11)) $RemoteNames
                if (-not [string]::IsNullOrWhiteSpace($c) -and (-not $candidates.Contains($c))) {
                    $candidates.Add($c)
                }
            }
        }
        if ($candidates.Count -eq 1) {
            Write-Host "💡 Detected sole local branch pointing at current HEAD as integration branch: '$($candidates[0])'" -ForegroundColor Cyan
            return $candidates[0]
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($DeclaredBranch)) {
        $cDeclared = Get-CleanBranchName $DeclaredBranch $RemoteNames
        if (-not [string]::IsNullOrWhiteSpace($cDeclared)) {
            Write-Host "💡 Selected integration branch registered in SKILL.md: '$cDeclared'" -ForegroundColor Cyan
            return $cDeclared
        }
    }

    if ($HasRemote) {
        $rHead = Invoke-Git @("symbolic-ref", "-q", "refs/remotes/origin/HEAD")
        if ($rHead.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($rHead.Output))) {
            $trimmed = $rHead.Output.Trim()
            if ($trimmed.StartsWith("refs/remotes/origin/")) {
                $rh = Get-CleanBranchName ($trimmed.Substring("refs/remotes/origin/".Length)) $RemoteNames
                if (-not [string]::IsNullOrWhiteSpace($rh)) {
                    Write-Host "💡 Selected integration branch from origin/HEAD: '$rh'" -ForegroundColor Cyan
                    return $rh
                }
            }
        }
        foreach ($b in @("main", "dev", "develop", "master", "trunk")) {
            if ((Invoke-Git @("rev-parse", "--verify", "origin/$b")).ExitCode -eq 0) {
                Write-Host "💡 Detected common remote integration branch: '$b'" -ForegroundColor Cyan
                return $b
            }
        }
    }

    foreach ($b in @("main", "dev", "develop", "master", "trunk")) {
        if ((Invoke-Git @("rev-parse", "--verify", $b)).ExitCode -eq 0) {
            Write-Host "💡 Detected common local integration branch: '$b'" -ForegroundColor Cyan
            return $b
        }
    }

    $dConf = Invoke-Git @("config", "init.defaultBranch")
    if ($dConf.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($dConf.Output))) {
        $cDef = Get-CleanBranchName ($dConf.Output.Trim()) $RemoteNames
        if (-not [string]::IsNullOrWhiteSpace($cDef)) {
            return $cDef
        }
    }
    return "main"
}

function Get-WorktreeMap {
    $wtList = Invoke-Git @("worktree", "list", "--porcelain")
    $map = @{}
    $allPaths = [System.Collections.Generic.List[string]]::new()
    if ($wtList.ExitCode -eq 0) {
        $lines = $wtList.Output -split "`r?`n"
        $currentPath = ""
        foreach ($line in $lines) {
            if ($line.StartsWith("worktree ")) {
                $currentPath = $line.Substring(9).Trim()
                try { $currentPath = (Resolve-Path $currentPath).Path } catch {}
                $allPaths.Add($currentPath)
            } elseif ($line.StartsWith("branch refs/heads/")) {
                $bName = $line.Substring(18).Trim()
                if ($currentPath -and $bName) {
                    $map[$bName] = $currentPath
                }
            } elseif ([string]::IsNullOrWhiteSpace($line)) {
                $currentPath = ""
            }
        }
    }
    return [PSCustomObject]@{
        BranchToPath = $map
        AllPaths     = $allPaths
    }
}

function Get-DirtyItems {
    param(
        [System.Collections.Generic.List[string]]$AllWorktreePaths,
        [System.Collections.Generic.List[string]]$DeclaredPatterns,
        [string]$WorkingDir = ""
    )
    $cmd = if ($WorkingDir) { @("-C", $WorkingDir, "status", "--porcelain") } else { @("status", "--porcelain") }
    $res = Invoke-Git $cmd
    if ($res.ExitCode -ne 0) {
        throw "Failed to get Git status: $($res.Error)"
    }
    $dirty = [System.Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($res.Output)) {
        $currentResolved = if ($WorkingDir) { (Resolve-Path $WorkingDir).Path } else { (Resolve-Path (Get-Location).Path).Path }
        foreach ($sl in ($res.Output -split "`r?`n")) {
            if ([string]::IsNullOrWhiteSpace($sl)) { continue }
            $relPath = $sl.Substring(3).Trim().Trim('"').TrimEnd('/')
            $absCandidate = Join-Path $currentResolved $relPath
            try { if (Test-Path $absCandidate) { $absCandidate = (Resolve-Path $absCandidate).Path } } catch {}

            $isRegisteredWt = $false
            foreach ($wt in $AllWorktreePaths) {
                if ($wt -eq $absCandidate) {
                    $isRegisteredWt = $true
                    break
                }
            }
            if ($isRegisteredWt) { continue }

            $isIgnored = $false
            foreach ($pat in $DeclaredPatterns) {
                if ($relPath -match $pat) {
                    $isIgnored = $true
                    break
                }
            }
            if ($isIgnored) { continue }

            $dirty.Add($sl)
        }
    }
    return $dirty
}

$skillDocPath = Join-Path $PSScriptRoot "..\SKILL.md"
if (-not (Test-Path $skillDocPath)) {
    $candidate = Join-Path (Get-Location).Path ".agents\skills\git-workflow\SKILL.md"
    if (Test-Path $candidate) {
        $skillDocPath = $candidate
    }
}
$declaredDirtyPatterns = [System.Collections.Generic.List[string]]::new()
if (Test-Path $skillDocPath) {
    try {
        $skillDocContent = [System.IO.File]::ReadAllText($skillDocPath, [System.Text.Encoding]::UTF8)
        $patRegex = '(?m)^[-*]\s*(?:\u5ffd\u7565\u672a\u8ffd\u8e2a\u8def\u5f84\u6b63\u5219|\u5ffd\u7565\u8def\u5f84\u6b63\u5219|\u5ffd\u7565\u6b63\u5219|Dirty ignore regex|Dirty ignore pattern)[\uFF1A:]\s*`?([^\r\n`]+)`?'
        $patMatches = [regex]::Matches($skillDocContent, $patRegex)
        foreach ($m in $patMatches) {
            $patVal = $m.Groups[1].Value.Trim()
            if (-not [string]::IsNullOrWhiteSpace($patVal)) {
                $declaredDirtyPatterns.Add($patVal)
            }
        }
    } catch {}
}

$remoteNames = Get-RemoteNames
$hasRemote = ((Invoke-Git @("remote", "get-url", "origin")).ExitCode -eq 0)
if ([string]::IsNullOrWhiteSpace($TargetCommit)) {
    $TargetCommit = Resolve-IntegrationBranch "" "" $hasRemote $remoteNames
}

$targetShaRes = Invoke-Git @("rev-parse", $TargetCommit)
if ($targetShaRes.ExitCode -ne 0) {
    throw "Failed to resolve target commit '$TargetCommit': $($targetShaRes.Error)"
}
$targetSha = $targetShaRes.Output
$targetShaShort = $targetSha.Substring(0, [Math]::Min(7, $targetSha.Length))

$wtInfo = Get-WorktreeMap
$currentDir = (Get-Location).Path
try { $currentDir = (Resolve-Path $currentDir).Path } catch {}

# Parse list of branches to align
$branchesToAlign = [System.Collections.Generic.List[string]]::new()
if ($Branches.Count -gt 0) {
    foreach ($b in $Branches) {
        $clean = Get-CleanBranchName $b $remoteNames
        if (-not [string]::IsNullOrWhiteSpace($clean)) {
            $branchesToAlign.Add($clean)
        }
    }
} else {
    $allLocalRefs = Invoke-Git @("for-each-ref", "--format=%(refname)", "refs/heads/")
    if ($allLocalRefs.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($allLocalRefs.Output))) {
        foreach ($line in ($allLocalRefs.Output -split "`r?`n")) {
            $lineClean = $line.Trim()
            if ($lineClean.StartsWith("refs/heads/")) {
                $rawB = $lineClean.Substring(11).Trim()
                $cleanB = Get-CleanBranchName $rawB $remoteNames
                if (-not [string]::IsNullOrWhiteSpace($cleanB) -and $cleanB -ne $TargetCommit) {
                    $branchesToAlign.Add($cleanB)
                }
            }
        }
    }
}

if ($branchesToAlign.Count -eq 0) {
    Write-Host "ℹ️ No branches require alignment." -ForegroundColor Yellow
    return
}

Write-Host ""
Write-Host "========================== Batch Branch Alignment ==========================" -ForegroundColor Cyan
Write-Host "Target baseline:   $TargetCommit ($targetShaShort)" -ForegroundColor Green
Write-Host "Branches to align: $($branchesToAlign.Count)" -ForegroundColor White
Write-Host "----------------------------------------------------------------------------"

$previewList = [System.Collections.Generic.List[PSCustomObject]]::new()
foreach ($b in $branchesToAlign) {
    $currSha = (Invoke-Git @("rev-parse", "--short", $b)).Output
    $isWt = $wtInfo.BranchToPath.ContainsKey($b)
    $wtPath = if ($isWt) { $wtInfo.BranchToPath[$b] } else { "-" }
    $action = if ($isWt) { "Worktree Reset" } else { "Branch Ref Update" }

    $previewList.Add([PSCustomObject]@{
        Branch   = $b
        Current  = $currSha
        Action   = $action
        Worktree = $wtPath
    })
}

$fmt = "{0,-22} {1,-10} {2,-20} {3}"
Write-Host ([string]::Format($fmt, "Branch Name", "Current SHA", "Reset Action", "Worktree Path")) -ForegroundColor DarkGray
Write-Host ("-" * 68) -ForegroundColor DarkGray
foreach ($item in $previewList) {
    Write-Host ([string]::Format($fmt, $item.Branch, $item.Current, $item.Action, $item.Worktree)) -ForegroundColor White
}
Write-Host "----------------------------------------------------------------------------"

if (-not $Apply) {
    Write-Host "⚠️ Currently in dry run mode. No resets or pushes were executed." -ForegroundColor Yellow
    Write-Host "To execute alignment and push, run with -Apply:" -ForegroundColor Cyan
    Write-Host "  pwsh .agents/skills/git-workflow/scripts/align-branches.ps1 -TargetCommit '$TargetCommit' -Apply" -ForegroundColor White
    Write-Host "============================================================================" -ForegroundColor Cyan
    Write-Host ""
    return
}

Write-Host ">> Executing batch alignment..." -ForegroundColor Cyan
$dirtyCurr = Get-DirtyItems $wtInfo.AllPaths $declaredDirtyPatterns
if ($dirtyCurr.Count -gt 0) {
    throw "Current workspace has uncommitted changes. Alignment aborted:`n$($dirtyCurr -join "`n")"
}
foreach ($item in $previewList) {
    if ($item.Worktree -ne "-" -and $item.Worktree -ne $currentDir) {
        $wtDirty = Get-DirtyItems $wtInfo.AllPaths $declaredDirtyPatterns $item.Worktree
        if ($wtDirty.Count -gt 0) {
            throw "Worktree ($($item.Worktree)) occupying branch '$($item.Branch)' has uncommitted changes. Alignment aborted:`n$($wtDirty -join "`n")"
        }
    }
}
foreach ($item in $previewList) {
    $b = $item.Branch
    if ($wtInfo.BranchToPath.ContainsKey($b)) {
        $wPath = $wtInfo.BranchToPath[$b]
        if ($wPath -eq $currentDir) {
            $r = Invoke-Git @("reset", "--hard", $targetSha)
            if ($r.ExitCode -ne 0) { throw "Resetting current workspace failed: $($r.Error)" }
        } else {
            $r = Invoke-Git @("-C", $wPath, "reset", "--hard", $targetSha)
            if ($r.ExitCode -ne 0) { throw "Resetting Worktree ($wPath) failed: $($r.Error)" }
        }
    } else {
        $r = Invoke-Git @("branch", "-f", $b, $targetSha)
        if ($r.ExitCode -ne 0) { throw "Updating branch ref $b failed: $($r.Error)" }
    }
    Write-Host "  ✅ $b aligned to $targetShaShort" -ForegroundColor Green
}

if ($hasRemote -and (-not $NoPush)) {
    Write-Host ">> Safely pushing aligned branches to remote (--force-with-lease)..." -ForegroundColor Cyan
    $pushBranches = @($branchesToAlign | Where-Object { Test-ValidBranchName $_ $remoteNames })
    $pushArgs = @("push", "--force-with-lease", "origin") + $pushBranches
    $pRes = Invoke-Git $pushArgs
    if ($pRes.ExitCode -ne 0) {
        Write-Warning "Batch push warning: $($pRes.Error); attempting per-branch safe push..."
        foreach ($b in $branchesToAlign) {
            $subPush = Invoke-Git @("push", "--force-with-lease", "origin", $b)
            if ($subPush.ExitCode -eq 0) {
                Write-Host "  ✅ $b pushed to origin" -ForegroundColor Green
            } else {
                Write-Warning "  ⚠️ $b push failed: $($subPush.Error)"
            }
        }
    } else {
        Write-Host "  ✅ All $($branchesToAlign.Count) branches safely pushed to origin" -ForegroundColor Green
    }
}

Write-Host "================== Batch Branch Alignment Completed ==================" -ForegroundColor Green
Write-Host ""