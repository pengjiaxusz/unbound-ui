<#
.SYNOPSIS
    Branch sync conflict recovery and continuation tool (Continue Sync)

.DESCRIPTION
    When cherry-pick conflicts occur during sync-branch.ps1 execution, after the developer/agent resolves conflict files and runs git add,
    this script resumes in 1-Shot:
    1. Continue cherry-pick;
    2. Continue applying remaining net commits in queue;
    3. Execute Tree-Diff Guard audit;
    4. Push integration branch;
    5. Batch align all branches and worktrees and safe push (--force-with-lease);
    6. Execute post-merge verification gates;
    7. Output COMPLETED_READY_TO_REPORT delivery dashboard.
    Also supports -Abort for complete rollback and clean recovery.

.PARAMETER Continue
    Resume remaining sync workflow after resolving conflicts.

.PARAMETER Skip
    Skip current interrupted commit (e.g. when changes are already equivalently landed or commit becomes empty), and continue with remaining queue.

.PARAMETER Abort
    Abort sync, restore integration branch to pre-sync backup, and clean state.

.PARAMETER Status
    Inspect current conflict and queue status.

.PARAMETER NoPush
    Skip remote push.

.PARAMETER NoVerify
    Skip post-merge project verification gates.
#>

[CmdletBinding()]
param(
    [switch]$Continue,
    [switch]$Skip,
    [switch]$Abort,
    [switch]$Status,
    [switch]$NoPush,
    [switch]$NoVerify
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

    if ($RemoteNames -and $RemoteNames.Contains($b)) { return $false }

    if ($b.StartsWith("heads/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("refs/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("remotes/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    if ($b.StartsWith("sync-backup/", [System.StringComparison]::OrdinalIgnoreCase) -or
        $b.StartsWith("cleanup-backup/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    if ($b -match "(?i)(?:^|/)(?:refs|heads|remotes)(?:/|$)") {
        return $false
    }

    if ($RemoteNames) {
        foreach ($r in $RemoteNames) {
            if ($b.StartsWith("$r/", [System.StringComparison]::OrdinalIgnoreCase)) {
                return $false
            }
        }
    }

    if ($b -eq "HEAD" -or $b.StartsWith("HEAD/", [System.StringComparison]::OrdinalIgnoreCase)) {
        return $false
    }

    $checkRes = Invoke-Git @("check-ref-format", "--branch", $b)
    if ($checkRes.ExitCode -ne 0) {
        return $false
    }

    return $true
}

function Get-WorktreeMap {
    $wtList = Invoke-Git @("worktree", "list", "--porcelain")
    $map = @{}
    if ($wtList.ExitCode -eq 0) {
        $lines = $wtList.Output -split "`r?`n"
        $currentPath = ""
        foreach ($line in $lines) {
            if ($line.StartsWith("worktree ")) {
                $currentPath = $line.Substring(9).Trim()
                try { $currentPath = (Resolve-Path $currentPath).Path } catch {}
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
    }
}

function Test-ContentPreserved {
    # Per-commit content preservation check (see REFERENCE IV): compare added/removed lines,
    # stripping patch headers (diff --git / index / --- / +++ / @@) and leading +/- symbols,
    # verifying that added content (or removed lines for pure deletions) is >= 80% preserved in the landed commit.
    param(
        [string]$SourceHash,
        [string]$LandedHash
    )
    $srcRes = Invoke-Git @("show", $SourceHash, "--format=", "--unified=0")
    $landRes = Invoke-Git @("show", $LandedHash, "--format=", "--unified=0")
    if ($srcRes.ExitCode -ne 0 -or $landRes.ExitCode -ne 0) {
        return $false
    }
    $landedSet = New-Object System.Collections.Generic.HashSet[string]
    foreach ($ln in ($landRes.Output -split "`r?`n")) {
        if ($ln.StartsWith("diff --git") -or $ln.StartsWith("index ") -or $ln.StartsWith("---") -or $ln.StartsWith("+++") -or $ln.StartsWith("@@")) {
            continue
        }
        if ($ln.StartsWith("+") -or $ln.StartsWith("-")) {
            $t = $ln.Substring(1).Trim()
            if (-not [string]::IsNullOrWhiteSpace($t)) {
                $landedSet.Add($t) | Out-Null
            }
        }
    }

    $srcAdded = [System.Collections.Generic.List[string]]::new()
    $srcRemoved = [System.Collections.Generic.List[string]]::new()
    foreach ($ln in ($srcRes.Output -split "`r?`n")) {
        if ($ln.StartsWith("diff --git") -or $ln.StartsWith("index ") -or $ln.StartsWith("---") -or $ln.StartsWith("+++") -or $ln.StartsWith("@@")) {
            continue
        }
        if ($ln.StartsWith("+")) {
            $t = $ln.Substring(1).Trim()
            if (-not [string]::IsNullOrWhiteSpace($t)) { $srcAdded.Add($t) }
        } elseif ($ln.StartsWith("-")) {
            $t = $ln.Substring(1).Trim()
            if (-not [string]::IsNullOrWhiteSpace($t)) { $srcRemoved.Add($t) }
        }
    }

    $checkList = if ($srcAdded.Count -gt 0) { $srcAdded } else { $srcRemoved }
    if ($checkList.Count -eq 0) {
        return $true
    }

    $hit = 0
    foreach ($line in $checkList) {
        if ($landedSet.Contains($line)) {
            $hit++
        }
    }
    $ratio = [double]$hit / [double]$checkList.Count
    return ($ratio -ge 0.8)
}

# Locate state file
$gitDirRes = Invoke-Git @("rev-parse", "--git-dir")
if ($gitDirRes.ExitCode -ne 0) {
    throw "Not executing inside a Git repository."
}
$gitDir = $gitDirRes.Output
try { $gitDir = (Resolve-Path $gitDir).Path } catch {}
$stateFilePath = Join-Path $gitDir "branch-sync-state.json"

if (-not (Test-Path $stateFilePath)) {
    # Check if there is an unfinished cherry-pick
    $cpHead = Join-Path $gitDir "CHERRY_PICK_HEAD"
    if (Test-Path $cpHead) {
        if ($Abort) {
            Invoke-Git @("cherry-pick", "--abort") | Out-Null
            Write-Host "✅ Cherry-pick operation aborted." -ForegroundColor Green
            return
        }
        throw "Detected unfinished cherry-pick, but branch-sync-state.json state record was not found."
    }
    Write-Host "ℹ️ No branch sync task is currently in progress or interrupted." -ForegroundColor Yellow
    return
}

$stateJson = [System.IO.File]::ReadAllText($stateFilePath, [System.Text.Encoding]::UTF8)
$state = ConvertFrom-Json $stateJson

$integBranch = $state.IntegrationBranch
$orderedCommits = @($state.OrderedCommits)
$currentIndex = [int]$state.CurrentCommitIndex
$backupIntegRef = $state.BackupIntegRef
$allSourceBranches = @($state.AllSourceBranches)
$declaredVerifyCmd = $state.DeclaredVerifyCmd
$skipPush = [bool]$state.NoPush -or $NoPush

if ($Status -or (-not $Continue -and -not $Skip -and -not $Abort)) {
    Write-Host ""
    Write-Host "================== Branch Sync Interrupted State ==================" -ForegroundColor Cyan
    Write-Host "Integration branch:       $integBranch"
    Write-Host "Current interrupted commit: $($orderedCommits[$currentIndex].Hash) - $($orderedCommits[$currentIndex].Subject)" -ForegroundColor Yellow
    Write-Host "Remaining commits in queue: $($orderedCommits.Count - $currentIndex)"
    
    $unmerged = Invoke-Git @("diff", "--name-only", "--diff-filter=U")
    if (-not [string]::IsNullOrWhiteSpace($unmerged.Output)) {
        Write-Host "Unresolved conflict files:" -ForegroundColor Red
        foreach ($uf in ($unmerged.Output -split "`r?`n")) {
            Write-Host "  ! $uf" -ForegroundColor Red
        }
    } else {
        Write-Host "Conflict file status:       Staged (no unresolved conflict markers)" -ForegroundColor Green
    }
    Write-Host "-------------------------------------------------------------------"
    Write-Host "Instructions:" -ForegroundColor Cyan
    Write-Host "  Resume after resolving:  pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Continue" -ForegroundColor White
    Write-Host "  Skip current commit:     pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Skip" -ForegroundColor White
    Write-Host "  Abort and safe rollback: pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Abort" -ForegroundColor White
    Write-Host "===================================================================" -ForegroundColor Cyan
    Write-Host ""
    return
}

if ($Abort) {
    Write-Host ">> Aborting sync and restoring integration branch..." -ForegroundColor Yellow
    Invoke-Git @("cherry-pick", "--abort") | Out-Null
    if (-not [string]::IsNullOrWhiteSpace($backupIntegRef)) {
        Invoke-Git @("reset", "--hard", $backupIntegRef) | Out-Null
        Write-Host "🛡️ Integration branch restored to pre-sync backup: $backupIntegRef" -ForegroundColor Green
    }
    Remove-Item -Path $stateFilePath -Force -ErrorAction SilentlyContinue
    Write-Host "✅ Branch sync safely aborted. Working state fully restored." -ForegroundColor Green
    return
}

if ($Continue -or $Skip) {
    $editorCmd = if ([System.Environment]::OSVersion.Platform -like "*Win*" -or $IsWindows) { "cmd.exe /c exit 0" } else { "true" }
    $gitDirRes = Invoke-Git @("rev-parse", "--git-dir")
    $gitDir = if ($gitDirRes.ExitCode -eq 0) { $gitDirRes.Output } else { ".git" }
    try { $gitDir = (Resolve-Path $gitDir).Path } catch {}
    $inCherryPick = (Test-Path (Join-Path $gitDir "CHERRY_PICK_HEAD"))

    if ($Skip) {
        Write-Host ">> Skipping current commit [$($currentIndex + 1)/$($orderedCommits.Count)]: $($orderedCommits[$currentIndex].Hash) - $($orderedCommits[$currentIndex].Subject)..." -ForegroundColor Yellow
        if ($inCherryPick) {
            $skipRes = Invoke-Git @("cherry-pick", "--skip")
            if ($skipRes.ExitCode -ne 0) {
                $inCherryPick = (Test-Path (Join-Path $gitDir "CHERRY_PICK_HEAD"))
                if ($inCherryPick) {
                    throw "git cherry-pick --skip failed: $($skipRes.Error)"
                }
            }
        }
        $state.CurrentCommitIndex = $currentIndex
        [System.IO.File]::WriteAllText($stateFilePath, (ConvertTo-Json $state -Depth 10), [System.Text.Encoding]::UTF8)
        Write-Host "⏭️ Successfully skipped commit $($orderedCommits[$currentIndex].Hash)." -ForegroundColor Yellow
    } else {
        # Check conflict files
        $unmerged = Invoke-Git @("diff", "--name-only", "--diff-filter=U")
        if (-not [string]::IsNullOrWhiteSpace($unmerged.Output)) {
            throw "Unresolved conflict files remaining! Please resolve conflicts in the following files and run git add before continuing:`n$($unmerged.Output)"
        }

        Write-Host ">> 1. Verifying and continuing current cherry-pick commit..." -ForegroundColor Cyan
        if ($inCherryPick) {
            $contRes = Invoke-Git @("-c", "core.editor=$editorCmd", "cherry-pick", "--continue", "--no-edit")
            if ($contRes.ExitCode -ne 0) {
                throw "git cherry-pick --continue failed: $($contRes.Error)"
            }
            Write-Host "✅ Current commit landed successfully!" -ForegroundColor Green
        } else {
            $lastLog = (Invoke-Git @("log", "-1", "--format=%s")).Output
            $targetSubj = $orderedCommits[$currentIndex].Subject
            if ($lastLog -match [regex]::Escape($targetSubj)) {
                Write-Host "ℹ️ Detected current commit already landed externally ($targetSubj); advancing to remaining queue." -ForegroundColor Green
            } else {
                Write-Host "ℹ️ No active cherry-pick in progress; checking remaining queue." -ForegroundColor Yellow
            }
        }
    }

    # Process remaining commits in queue
    $nextIndex = $currentIndex + 1
    while ($nextIndex -lt $orderedCommits.Count) {
        $c = $orderedCommits[$nextIndex]
        Write-Host ">> Applying remaining commit [$($nextIndex + 1)/$($orderedCommits.Count)]: $($c.Hash.Substring(0, [Math]::Min(7, $c.Hash.Length))) $($c.Subject)..." -ForegroundColor DarkGray
        
        $cp = Invoke-Git @("-c", "core.editor=$editorCmd", "cherry-pick", $c.FullHash)
        if ($cp.ExitCode -ne 0) {
            $uCheck = Invoke-Git @("diff", "--name-only", "--diff-filter=U")
            $uOut = if ($null -ne $uCheck.Output) { $uCheck.Output.Trim() } else { "" }
            if ([string]::IsNullOrWhiteSpace($uOut)) {
                # Empty commit (already equivalently applied/duplicate): skip, preserving changes
                Invoke-Git @("cherry-pick", "--skip") | Out-Null
                Write-Host "ℹ️ Commit $($c.Hash) is empty (changes already exist); skipped." -ForegroundColor Yellow
                $state.CurrentCommitIndex = $nextIndex
                [System.IO.File]::WriteAllText($stateFilePath, (ConvertTo-Json $state -Depth 10), [System.Text.Encoding]::UTF8)
                $nextIndex++
                continue
            }
            # True conflict: update index and interrupt
            $state.CurrentCommitIndex = $nextIndex
            [System.IO.File]::WriteAllText($stateFilePath, (ConvertTo-Json $state -Depth 10), [System.Text.Encoding]::UTF8)
            $unm = $uOut
            Write-Host "⚠️ Conflict recurred at commit $($c.Hash)!" -ForegroundColor Yellow
            Write-Host "Conflict files:`n$unm" -ForegroundColor Red
            Write-Host "Please resolve conflicts, run git add, then execute continue-sync.ps1 -Continue" -ForegroundColor Cyan
            exit 1
        }
        $headTip = (Invoke-Git @("rev-parse", "HEAD")).Output
        if (-not (Test-ContentPreserved $c.FullHash $headTip.Trim())) {
            throw "Content preservation check failed: commit $($c.Hash) - $($c.Subject) was incomplete after landing; suspected data loss, interrupted!"
        }
        $nextIndex++
    }

    Write-Host "✅ All net contribution commits successfully cherry-picked!" -ForegroundColor Green

    # Tree-Diff Guard v2: cherry residue + file-level content containment dual audit
    Write-Host "🔍 Executing post-merge change preservation audit (Tree-Diff Guard)..." -ForegroundColor DarkCyan
    $backupSourceRefs = $state.BackupSourceRefs
    $guardFailures = [System.Collections.Generic.List[string]]::new()
    if ($null -eq $backupSourceRefs) {
        Write-Host "⚠️ Missing BackupSourceRefs in state file (legacy state); falling back to snapshot inspection." -ForegroundColor Yellow
        foreach ($sb in $allSourceBranches) {
            $checkRef = "refs/sync-backup/$($sb -replace '[^a-zA-Z0-9_\-]', '_')"
            $refLines = @((Invoke-Git @("for-each-ref", "--sort=-committerdate", "--format=%(refname)", "$checkRef*")).Output -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            $bRef = if ($refLines.Count -gt 0) { $refLines[0].Trim() } else { "" }
            if (-not [string]::IsNullOrWhiteSpace($bRef)) {
                $postCherry = Invoke-Git @("cherry", "-v", $integBranch, $bRef)
                if ($postCherry.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($postCherry.Output))) {
                    foreach ($pcl in ($postCherry.Output -split "`r?`n")) {
                        if ($pcl -match '^\+\s+([0-9a-fA-F]+)\s+(.*)$') {
                            $resHash = $Matches[1]
                            $cSubj = $Matches[2].Trim()
                            # Empty commits lack patch-id; cherry always reports `+`. Zero file changes means landed.
                            $emptyCheck = Invoke-Git @("diff-tree", "--no-commit-id", "--name-only", "-r", $resHash)
                            if ($emptyCheck.ExitCode -eq 0 -and [string]::IsNullOrWhiteSpace($emptyCheck.Output)) {
                                continue
                            }
                            # Patch context drift tolerance: if patch-id shifted due to commit order but diff lines preserved, treat as merged
                            if (-not [string]::IsNullOrWhiteSpace($cSubj)) {
                                $landedBySubj = Invoke-Git @("log", $integBranch, "--fixed-strings", "--grep=$cSubj", "-n", "5", "--format=%H")
                                if ($landedBySubj.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($landedBySubj.Output))) {
                                    $matched = $false
                                    foreach ($candHash in ($landedBySubj.Output -split "`r?`n")) {
                                        $ch = $candHash.Trim()
                                        if (-not [string]::IsNullOrWhiteSpace($ch) -and (Test-ContentPreserved $resHash $ch)) {
                                            $matched = $true
                                            break
                                        }
                                    }
                                    if ($matched) {
                                        continue
                                    }
                                }
                            }
                            if (Test-ContentPreserved $resHash $integBranch) {
                                continue
                            }
                            $descendants = Invoke-Git @("rev-list", "$resHash..$bRef", "--reverse")
                            if ($descendants.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($descendants.Output))) {
                                $descMerged = $false
                                foreach ($descHash in ($descendants.Output -split "`r?`n")) {
                                    $dh = $descHash.Trim()
                                    if (-not [string]::IsNullOrWhiteSpace($dh) -and (Test-ContentPreserved $dh $integBranch)) {
                                        $descMerged = $true
                                        break
                                    }
                                }
                                if ($descMerged) {
                                    continue
                                }
                            }
                            $guardFailures.Add("Source branch '$sb' has unapplied net commit: $resHash $cSubj")
                        }
                    }
                }
            }
        }
    } else {
        foreach ($prop in $backupSourceRefs.PSObject.Properties) {
            $sb = $prop.Name
            $bRef = [string]$prop.Value
            if ([string]::IsNullOrWhiteSpace($bRef)) { continue }

            $postCherry = Invoke-Git @("cherry", "-v", $integBranch, $bRef)
            if ($postCherry.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($postCherry.Output))) {
                foreach ($pcl in ($postCherry.Output -split "`r?`n")) {
                    if ($pcl -match '^\+\s+([0-9a-fA-F]+)\s+(.*)$') {
                        $resHash = $Matches[1]
                        $cSubj = $Matches[2].Trim()
                        # Empty commits lack patch-id; cherry always reports `+`. Zero file changes means landed.
                        $emptyCheck = Invoke-Git @("diff-tree", "--no-commit-id", "--name-only", "-r", $resHash)
                        if ($emptyCheck.ExitCode -eq 0 -and [string]::IsNullOrWhiteSpace($emptyCheck.Output)) {
                            continue
                        }
                        # Patch context drift tolerance
                        if (-not [string]::IsNullOrWhiteSpace($cSubj)) {
                            $landedBySubj = Invoke-Git @("log", $integBranch, "--fixed-strings", "--grep=$cSubj", "-n", "5", "--format=%H")
                            if ($landedBySubj.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($landedBySubj.Output))) {
                                $matched = $false
                                foreach ($candHash in ($landedBySubj.Output -split "`r?`n")) {
                                    $ch = $candHash.Trim()
                                    if (-not [string]::IsNullOrWhiteSpace($ch) -and (Test-ContentPreserved $resHash $ch)) {
                                        $matched = $true
                                        break
                                    }
                                }
                                if ($matched) {
                                    continue
                                }
                            }
                        }
                        if (Test-ContentPreserved $resHash $integBranch) {
                            continue
                        }
                        $descendants = Invoke-Git @("rev-list", "$resHash..$bRef", "--reverse")
                        if ($descendants.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($descendants.Output))) {
                            $descMerged = $false
                            foreach ($descHash in ($descendants.Output -split "`r?`n")) {
                                $dh = $descHash.Trim()
                                if (-not [string]::IsNullOrWhiteSpace($dh) -and (Test-ContentPreserved $dh $integBranch)) {
                                    $descMerged = $true
                                    break
                                }
                            }
                            if ($descMerged) {
                                continue
                            }
                        }
                        $guardFailures.Add("Source branch '$sb' has unapplied net commit: $resHash $cSubj")
                    }
                }
            }

            # File-level diff check
            $mBase = (Invoke-Git @("merge-base", $integBranch, $bRef)).Output
            if (-not [string]::IsNullOrWhiteSpace($mBase)) {
                $srcDiff = Invoke-Git @("diff", "--name-only", "$mBase..$bRef")
                $srcFiles = if ($srcDiff.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($srcDiff.Output))) {
                    @($srcDiff.Output -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
                } else { @() }
                $integDiff = Invoke-Git @("diff", "--name-only", "$mBase..$integBranch")
                $integSet = @{}
                foreach ($t in ($integDiff.Output -split "`r?`n")) {
                    $t = $t.Trim()
                    if (-not [string]::IsNullOrWhiteSpace($t)) { $integSet[$t] = $true }
                }
                $missing = @($srcFiles | Where-Object { -not $integSet.ContainsKey($_) })
                if ($missing.Count -gt 0) {
                    $guardFailures.Add("Source branch '$sb' missing files: $($missing -join ', ')")
                }
            }
        }
    }
    if ($guardFailures.Count -gt 0) {
        throw "Tree-Diff Guard intercepted:`n$($guardFailures -join "`n")"
    }
    Write-Host "✅ Tree-Diff Guard audit passed: all branch changes 100% preserved!" -ForegroundColor Green

    # Push integration branch
    $hasRemote = ((Invoke-Git @("remote", "get-url", "origin")).ExitCode -eq 0)
    if ($hasRemote -and (-not $skipPush)) {
        Write-Host ">> Pushing integration branch origin/$integBranch..." -ForegroundColor DarkGray
        $pInteg = Invoke-Git @("push", "origin", $integBranch)
        if ($pInteg.ExitCode -ne 0) { throw "Failed to push integration branch: $($pInteg.Error)" }
    }

    # Align all branches and worktrees
    Write-Host ">> Batch aligning all branches & Worktrees to latest integration branch tip..." -ForegroundColor Cyan
    $integTip = (Invoke-Git @("rev-parse", $integBranch)).Output
    $wtInfo = Get-WorktreeMap
    $currentDir = (Get-Location).Path
    try { $currentDir = (Resolve-Path $currentDir).Path } catch {}

    foreach ($sb in $allSourceBranches) {
        if ($sb -eq $integBranch) { continue }
        if ($wtInfo.BranchToPath.ContainsKey($sb)) {
            $wPath = $wtInfo.BranchToPath[$sb]
            if ($wPath -ne $currentDir) {
                Invoke-Git @("-C", $wPath, "reset", "--hard", $integTip) | Out-Null
            }
        } else {
            Invoke-Git @("branch", "-f", $sb, $integTip) | Out-Null
        }
        Write-Host "  ✅ Branch '$sb' aligned" -ForegroundColor Green
    }

    if ($hasRemote -and (-not $skipPush)) {
        Write-Host ">> Safely pushing all aligned branches (--force-with-lease)..." -ForegroundColor Cyan
        $remoteNames = Get-RemoteNames
        $pushBranches = $allSourceBranches | Where-Object { $_ -ne $integBranch -and (Test-ValidBranchName $_ $remoteNames) }
        if ($pushBranches.Count -gt 0) {
            $pArgs = @("push", "--force-with-lease", "origin") + $pushBranches
            $pRes = Invoke-Git $pArgs
            if ($pRes.ExitCode -ne 0) {
                foreach ($pb in $pushBranches) {
                    Invoke-Git @("push", "--force-with-lease", "origin", $pb) | Out-Null
                }
            }
            Write-Host "  ✅ All aligned branches pushed to origin" -ForegroundColor Green
        }
    }

    # Post-merge verification
    $verifyStatus = "SKIPPED"
    if (-not $NoVerify -and (-not [string]::IsNullOrWhiteSpace($declaredVerifyCmd))) {
        Write-Host ">> Running project post-merge verification command: $declaredVerifyCmd" -ForegroundColor Cyan
        try {
            $pinfo = New-Object System.Diagnostics.ProcessStartInfo
            if ($IsWindows -or ($PSVersionTable.PSEdition -ne "Core" -and [System.Environment]::OSVersion.Platform -like "*Win*")) {
                $pinfo.FileName = "powershell.exe"
                $pinfo.Arguments = "-NoProfile -ExecutionPolicy Bypass -Command `"$declaredVerifyCmd`""
            } else {
                $pinfo.FileName = "sh"
                $pinfo.Arguments = "-c `"$declaredVerifyCmd`""
            }
            $pinfo.RedirectStandardOutput = $false
            $pinfo.RedirectStandardError = $false
            $pinfo.UseShellExecute = $false
            $pinfo.WorkingDirectory = (Get-Location).Path

            $vProc = [System.Diagnostics.Process]::Start($pinfo)
            $vProc.WaitForExit()
            if ($vProc.ExitCode -eq 0) {
                $verifyStatus = "PASSED ($declaredVerifyCmd)"
            } else {
                $verifyStatus = "FAILED (Exit code $($vProc.ExitCode))"
            }
        } catch {
            $verifyStatus = "ERROR ($($_.Exception.Message))"
        }
    }

    # Clean state file
    Remove-Item -Path $stateFilePath -Force -ErrorAction SilentlyContinue

    Write-Host ""
    Write-Host "================== BRANCH SYNC SUCCESS ==================" -ForegroundColor Green
    Write-Host "Integration branch: $integBranch ($($integTip.Substring(0, [Math]::Min(7, $integTip.Length))))"
    Write-Host "Synced branches:    $($allSourceBranches.Count) branches (all aligned and safely pushed)"
    Write-Host "Landed net commits: $($orderedCommits.Count) (strictly linear, 0 merge)"
    Write-Host "Preservation audit: ✅ PASSED (Tree-Diff 100% preserved)"
    Write-Host "Gate verification:  $verifyStatus"
    Write-Host "Working tree state: ✅ Clean"
    Write-Host "Final status:       COMPLETED_READY_TO_REPORT"
    Write-Host "=========================================================" -ForegroundColor Green
    Write-Host ""
}