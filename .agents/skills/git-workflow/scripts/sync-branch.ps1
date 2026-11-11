<#
.SYNOPSIS
    Worktree-aware fast linear multi-branch synchronization engine (Branch Sync Fast-Track Engine)

.DESCRIPTION
    Automates linear synchronization between integration branch and all/specified feature branches with 1-Shot closed loop:
    1. Automatic smart integration branch deduction: prioritizes active branch in current directory / worktree;
    2. Default all-branch sync mode: automatically scans and synchronizes all branches with net contributions when -SourceBranch is omitted;
    3. Global committerdate chronological ordering: avoids topological ordering conflicts during multi-branch merges;
    4. Strict workspace cleanliness checks: prevents dirty working tree loss (excluding registered worktrees and declared ignore patterns);
    5. Dual-end alignment safety checks (local vs origin topology, auto fast-forwarding, branch divergence detection);
    6. Automatic persistent safety backup refs (refs/sync-backup/...) for instant zero-loss rollback;
    7. Fast-forward rebase or ordered linear cherry-pick (zero merge commits); auto-persists state on conflict with continue-sync.ps1 resume;
    8. Tree-level change preservation audit (Tree-Diff Guard v2): verifies all branch diffs are 100% incorporated;
    9. Batch worktree-aware alignment and safe push (--force-with-lease);
    10. Automated post-merge project verification gates for 1-Shot delivery.

.PARAMETER SourceBranch
    Source feature branch to sync. If omitted, syncs all branches with net contributions.

.PARAMETER IntegrationBranch
    Target integration branch. Defaults in order: active branch -> worktree branch -> SKILL.md -> remote/local probe (main/dev/master) -> init.defaultBranch.

.PARAMETER DirtyIgnorePattern
    Additional regex patterns to ignore during dirty checks. Can also be declared in SKILL.md.

.PARAMETER All
    Explicitly sync all branches. Default behavior when -SourceBranch is omitted.

.PARAMETER Apply
    Execute actual merge, push, and alignment. Dry run by default.

.PARAMETER NoPush
    Do not push to remote on -Apply (for local rehearsals or offline environments).

.PARAMETER NoFetch
    Skip git fetch before execution.

.PARAMETER NoVerify
    Skip automated post-merge project verification command.

.EXAMPLE
    # 1-Shot fast execution (recommends: 1 call completes all merges, pushes, alignments, and gates)
    pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -Apply

    # Sync specific source branch
    pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -SourceBranch feat/my-feature -Apply

    # Fast inspection (read-only mode)
    pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$SourceBranch = "",

    [Parameter(Position = 1)]
    [string]$IntegrationBranch = "",

    [Parameter()]
    [string[]]$DirtyIgnorePattern = @(),

    [switch]$All,
    [switch]$Apply,
    [switch]$NoPush,
    [switch]$NoFetch,
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

function Assert-GitSuccess {
    param(
        [object]$Result,
        [string]$StepName
    )
    if ($Result.ExitCode -ne 0) {
        $msg = if ($Result.Error) { $Result.Error } else { $Result.Output }
        throw "[$StepName] Git command failed (exit code $($Result.ExitCode)): $msg"
    }
}

function Get-RemoteNames {
    $res = Invoke-Git @("remote")
    $names = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    if ($res.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($res.Output))) {
        foreach ($line in ($res.Output -split "`r?`n")) {
            $t = $line.Trim()
            if (-not [string]::IsNullOrWhiteSpace($t)) { $names.Add($t) | Out-Null }
        }
    }
    if ($names.Count -eq 0) { $names.Add("origin") | Out-Null }
    return $names
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

function Get-RemoteTruthMap {
    param([System.Collections.Generic.HashSet[string]]$RemoteNames = $null)
    # Remote truth discovery: trust only git ls-remote (unaffected by sandbox ref virtualization; see REFERENCE I).
    # Returns hashtable on success (branch name -> remote sha); returns $null on failure (no remote/offline) to fall back to legacy enumeration.
    # Never throws.
    try {
        $res = Invoke-Git @("ls-remote", "--heads", "origin")
        if ($res.ExitCode -ne 0) {
            return $null
        }
        $map = @{}
        if (-not [string]::IsNullOrWhiteSpace($res.Output)) {
            foreach ($line in ($res.Output -split "`r?`n")) {
                if ([string]::IsNullOrWhiteSpace($line)) { continue }
                $parts = $line -split "\s+"
                if ($parts.Length -ge 2) {
                    $sha = $parts[0].Trim()
                    $ref = $parts[1].Trim()
                    if ($ref.StartsWith("refs/heads/")) {
                        $rawB = $ref.Substring(11).Trim()
                        $bName = Get-CleanBranchName $rawB $RemoteNames
                        if ((-not [string]::IsNullOrWhiteSpace($bName)) -and (-not [string]::IsNullOrWhiteSpace($sha))) {
                            $map[$bName] = $sha
                        } elseif (-not [string]::IsNullOrWhiteSpace($rawB)) {
        Write-Warning "⚠️ Malformed remote branch ref '$ref' detected; safely ignored and will never create local tracking branch!"
                        }
                    }
                }
            }
        }
        return $map
    } catch {
        return $null
    }
}

function Get-CommitPatchId {
    # Compute patch-id (stable mode) for cross-branch commit deduplication.
    # Empty patches (merges/empty commits) return empty string; caller falls back to full hash as deduplication key.
    # Never throws.
    param(
        [string]$FullHash
    )
    try {
        $showRes = Invoke-Git @("show", $FullHash, "--format=")
        if ($showRes.ExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($showRes.Output)) {
            return ""
        }
        $pidOut = (($showRes.Output -split "`r?`n") | git patch-id --stable 2>$null | Out-String)
        if ([string]::IsNullOrWhiteSpace($pidOut)) {
            return ""
        }
        $token = ($pidOut.Trim() -split "\s+")[0].Trim()
        if ([string]::IsNullOrWhiteSpace($token) -or ($token -match "^0+$")) {
            return ""
        }
        return $token
    } catch {
        return ""
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
    $srcAddedCount = 0
    $srcAddedMatched = 0
    $currentFile = ""
    foreach ($ln in ($srcRes.Output -split "`r?`n")) {
        if ($ln -match "^diff --git a/.*? b/(.*)$") {
            $currentFile = $Matches[1].Trim()
            continue
        }
        if ($ln.StartsWith("index ") -or $ln.StartsWith("---") -or $ln.StartsWith("+++") -or $ln.StartsWith("@@")) {
            continue
        }
        if ($ln.StartsWith("+")) {
            $t = $ln.Substring(1).Trim()
            if ([string]::IsNullOrWhiteSpace($t)) {
                continue
            }
            $srcAddedCount++
            if ($landedSet.Contains($t)) {
                $srcAddedMatched++
            } elseif (-not [string]::IsNullOrWhiteSpace($currentFile)) {
                $needle = if ($t.Length -gt 512) { $t.Substring(0, 512) } else { $t }
                $checkPre = Invoke-Git @("grep", "-F", "-q", "-e", $needle, $LandedHash, "--", $currentFile)
                if ($checkPre.ExitCode -eq 0) {
                    $srcAddedMatched++
                }
            }
        }
    }
    if ($srcAddedCount -eq 0) {
        $delCount = 0
        $delMatched = 0
        $currentFile = ""
        foreach ($ln in ($srcRes.Output -split "`r?`n")) {
            if ($ln -match "^diff --git a/.*? b/(.*)$") {
                $currentFile = $Matches[1].Trim()
                continue
            }
            if ($ln.StartsWith("index ") -or $ln.StartsWith("---") -or $ln.StartsWith("+++") -or $ln.StartsWith("@@")) {
                continue
            }
            if ($ln.StartsWith("-")) {
                $t = $ln.Substring(1).Trim()
                if (-not [string]::IsNullOrWhiteSpace($t)) {
                    $delCount++
                    if ($landedSet.Contains($t)) {
                        $delMatched++
                    } elseif (-not [string]::IsNullOrWhiteSpace($currentFile)) {
                        $needle = if ($t.Length -gt 512) { $t.Substring(0, 512) } else { $t }
                        $checkPre = Invoke-Git @("grep", "-F", "-q", "-e", $needle, $LandedHash, "--", $currentFile)
                        if ($checkPre.ExitCode -ne 0) {
                            $delMatched++
                        }
                    }
                }
            }
        }
        if ($delCount -eq 0) { return $true }
        return ($delMatched / $delCount) -ge 0.8
    }
    return ($srcAddedMatched / $srcAddedCount) -ge 0.8
}

function Test-BranchFullyMerged {
    # Tree-Diff Guard v2 (see REFERENCE IV): dual verification.
    # (i) cherry residue: git cherry -v <IntegrationBranch> <source_snapshot> must have 0 '+' lines;
    # (ii) file-level content containment: changed files list from merge-base to source snapshot must be
    #      completely contained in changed files list from merge-base to integration branch.
    param(
        [string]$TargetBranch,
        [string]$SourceSnapshotRef
    )
    $missingCommits = [System.Collections.Generic.List[string]]::new()
    $missingFiles = [System.Collections.Generic.List[string]]::new()
    $cherryRes = Invoke-Git @("cherry", "-v", $TargetBranch, $SourceSnapshotRef)
    if ($cherryRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($cherryRes.Output))) {
        foreach ($pcl in ($cherryRes.Output -split "`r?`n")) {
            if ($pcl -match '^\+\s+([0-9a-fA-F]+)\s+(.*)$') {
                $resHash = $Matches[1]
                $cSubj = $Matches[2].Trim()
    # Empty commits (no file diff) lack patch-id; cherry always reports '+': empty diff considered landed.
                $emptyCheck = Invoke-Git @("diff-tree", "--no-commit-id", "--name-only", "-r", $resHash)
                if ($emptyCheck.ExitCode -eq 0 -and [string]::IsNullOrWhiteSpace($emptyCheck.Output)) {
                    continue
                }
    # Patch context drift tolerance (see REFERENCE IV): if patch-id shifted due to commit order,
    # but added/removed lines are 100% preserved (Test-ContentPreserved), treat as merged.
                if (-not [string]::IsNullOrWhiteSpace($cSubj)) {
                    $landedBySubj = Invoke-Git @("log", $TargetBranch, "--fixed-strings", "--grep=$cSubj", "-n", "5", "--format=%H")
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
                if (Test-ContentPreserved $resHash $TargetBranch) {
                    continue
                }
                $descendants = Invoke-Git @("rev-list", "$resHash..$SourceSnapshotRef", "--reverse")
                if ($descendants.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($descendants.Output))) {
                    $descMerged = $false
                    foreach ($descHash in ($descendants.Output -split "`r?`n")) {
                        $dh = $descHash.Trim()
                        if (-not [string]::IsNullOrWhiteSpace($dh) -and (Test-ContentPreserved $dh $TargetBranch)) {
                            $descMerged = $true
                            break
                        }
                    }
                    if ($descMerged) {
                        continue
                    }
                }
                $missingCommits.Add($pcl.Trim())
            }
        }
    }
    $mbRes = Invoke-Git @("merge-base", $TargetBranch, $SourceSnapshotRef)
    if ($mbRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($mbRes.Output))) {
        $mb = $mbRes.Output.Trim()
        $srcFilesRes = Invoke-Git @("diff", "--name-only", $mb, $SourceSnapshotRef)
        $integFilesRes = Invoke-Git @("diff", "--name-only", $mb, $TargetBranch)
        $srcFiles = @()
        if ($srcFilesRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($srcFilesRes.Output))) {
            $srcFiles = @($srcFilesRes.Output -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        }
        $integSet = New-Object System.Collections.Generic.HashSet[string]([System.StringComparer]::Ordinal)
        if ($integFilesRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($integFilesRes.Output))) {
            foreach ($f in ($integFilesRes.Output -split "`r?`n")) {
                $t = $f.Trim()
                if (-not [string]::IsNullOrWhiteSpace($t)) {
                    $integSet.Add($t) | Out-Null
                }
            }
        }
        foreach ($f in $srcFiles) {
            if (-not $integSet.Contains($f)) {
                $missingFiles.Add($f)
            }
        }
    }
    $merged = (($missingCommits.Count -eq 0) -and ($missingFiles.Count -eq 0))
    return [PSCustomObject]@{
        Merged         = $merged
        MissingCommits = $missingCommits
        MissingFiles   = $missingFiles
    }
}

# 1. Initialization and configuration
$remoteCheck = Invoke-Git @("remote", "get-url", "origin")
$hasRemote = ($remoteCheck.ExitCode -eq 0)

$skillDocPath = Join-Path $PSScriptRoot "..\SKILL.md"
if (-not (Test-Path $skillDocPath)) {
    $candidate = Join-Path (Get-Location).Path ".agents\skills\git-workflow\SKILL.md"
    if (Test-Path $candidate) {
        $skillDocPath = $candidate
    }
}
$declaredIntegrationBranch = ""
$declaredDirtyPatterns = [System.Collections.Generic.List[string]]::new()
$declaredVerifyCmd = ""

if (Test-Path $skillDocPath) {
    try {
        $skillDocContent = [System.IO.File]::ReadAllText($skillDocPath, [System.Text.Encoding]::UTF8)
        if ($skillDocContent -match '(?m)^[-*]\s*(?:\u96c6\u6210\u5206\u652f|Integration branch)[\uFF1A:]\s*`?([a-zA-Z0-9_\-\.\/]+)`?') {
            $declaredIntegrationBranch = $Matches[1].Trim()
        }
        $patRegex = '(?m)^[-*]\s*(?:\u5ffd\u7565\u672a\u8ffd\u8e2a\u8def\u5f84\u6b63\u5219|\u5ffd\u7565\u8def\u5f84\u6b63\u5219|\u5ffd\u7565\u6b63\u5219|Dirty ignore regex|Dirty ignore pattern)[\uFF1A:]\s*`?([^\r\n`]+)`?'
        $patMatches = [regex]::Matches($skillDocContent, $patRegex)
        foreach ($m in $patMatches) {
            $patVal = $m.Groups[1].Value.Trim()
            if (-not [string]::IsNullOrWhiteSpace($patVal)) {
                $declaredDirtyPatterns.Add($patVal)
            }
        }
        $verifyBlockRegex = '(?ms)###\s*(?:\u5408\u540e\u9a8c\u8bc1\u547d\u4ee4|Post-Merge Validation Command|\u5408\u540e\u68c0\u9a8c\u547d\u4ee4)\s*.*?(?:```(?:powershell|bash|sh|cmd|pwsh)?\r?\n(.*?)\r?\n```)'
        if ($skillDocContent -match $verifyBlockRegex) {
            $codeBlock = $Matches[1]
            $codeLines = $codeBlock -split "`r?`n"
            foreach ($cl in $codeLines) {
                $trimmed = $cl.Trim()
                if (-not [string]::IsNullOrWhiteSpace($trimmed) -and -not $trimmed.StartsWith("#")) {
                    $declaredVerifyCmd = $trimmed
                    break
                }
            }
        }
    } catch {}
}
foreach ($p in $DirtyIgnorePattern) {
    if (-not [string]::IsNullOrWhiteSpace($p)) {
        $declaredDirtyPatterns.Add($p.Trim())
    }
}

# 2. Resolve integration branch
$remoteNames = Get-RemoteNames
$IntegrationBranch = Resolve-IntegrationBranch $IntegrationBranch $declaredIntegrationBranch $hasRemote $remoteNames
$integCheck = Invoke-Git @("rev-parse", "--verify", $IntegrationBranch)
if ($integCheck.ExitCode -ne 0) {
    throw "Integration branch '$IntegrationBranch' does not exist; check the specified branch name."
}

$wtInfo = Get-WorktreeMap
$currentWorktreePath = (Get-Location).Path
try { $currentWorktreePath = (Resolve-Path $currentWorktreePath).Path } catch {}

# 3. Verify current workspace cleanliness
$dirtyItems = Get-DirtyItems $wtInfo.AllPaths $declaredDirtyPatterns
if ($dirtyItems.Count -gt 0) {
    if ($Apply) {
        $dirtyList = $dirtyItems -join "`n"
    throw "Current workspace has uncommitted changes. Commit or stash before running sync:`n$dirtyList"
    } else {
    Write-Host "⚠️ Warning: Current workspace has uncommitted changes (pre-flight displays topology; -Apply will block)." -ForegroundColor Yellow
    }
}

# 4. Fetch latest remote state
$doPush = $hasRemote -and (-not $NoPush)
if ($hasRemote -and (-not $NoFetch)) {
    Write-Host "Fetching latest remote refs (git fetch --prune)..." -ForegroundColor DarkGray
    Invoke-Git @("fetch", "--prune", "origin") | Out-Null
}

# 5. Determine sync mode: All-Branches (default) or Single-Branch
$isAllBranchesMode = [string]::IsNullOrWhiteSpace($SourceBranch) -or $All

if (-not $isAllBranchesMode) {
    # -----------------------------
# Single-Branch Mode
    # -----------------------------
    $SourceBranch = Get-CleanBranchName $SourceBranch $remoteNames
    if ([string]::IsNullOrWhiteSpace($SourceBranch)) { throw "Source branch '$SourceBranch' name is invalid or contains invalid path characters." }
    if ($SourceBranch -eq $IntegrationBranch) {
    throw "Source branch '$SourceBranch' is identical to integration branch '$IntegrationBranch'; self-merge not required."
    }

    $sourceCheck = Invoke-Git @("rev-parse", "--verify", $SourceBranch)
    if ($sourceCheck.ExitCode -ne 0) {
        if ($hasRemote) {
            $sourceRemoteCheck = Invoke-Git @("rev-parse", "--verify", "origin/$SourceBranch")
            if ($sourceRemoteCheck.ExitCode -ne 0) {
    throw "Source branch '$SourceBranch' does not exist locally or on remote origin."
            }
            Invoke-Git @("branch", "--track", $SourceBranch, "origin/$SourceBranch") | Out-Null
        } else {
    throw "Source branch '$SourceBranch' does not exist locally."
        }
    }

    $isOccupiedByOther = $false
    $occupiedWorktreePath = $null
    if ($wtInfo.BranchToPath.ContainsKey($SourceBranch)) {
        $p = $wtInfo.BranchToPath[$SourceBranch]
        if ($p -ne $currentWorktreePath) {
            $isOccupiedByOther = $true
            $occupiedWorktreePath = $p
        }
    }
    $route = if ($isOccupiedByOther) { "Route B" } else { "Route A" }

    if ($route -eq "Route B") {
        $wtDirty = Get-DirtyItems $wtInfo.AllPaths $declaredDirtyPatterns $occupiedWorktreePath
        if ($wtDirty.Count -gt 0) {
            $dirtyList = $wtDirty -join "`n"
    throw "Worktree ($occupiedWorktreePath) occupying source branch '$SourceBranch' has uncommitted changes. Commit or stash in that directory:`n$dirtyList"
        }
    }

    # Dual-end alignment check
    if ($hasRemote -and (-not $NoFetch)) {
        $hasRemoteSource = (Invoke-Git @("rev-parse", "--verify", "origin/$SourceBranch")).ExitCode -eq 0
        if ($hasRemoteSource) {
            $lSha = (Invoke-Git @("rev-parse", $SourceBranch)).Output
            $rSha = (Invoke-Git @("rev-parse", "origin/$SourceBranch")).Output
            if ($lSha -ne $rSha) {
                $isAnc = (Invoke-Git @("merge-base", "--is-ancestor", $lSha, $rSha)).ExitCode -eq 0
                $isRAnc = (Invoke-Git @("merge-base", "--is-ancestor", $rSha, $lSha)).ExitCode -eq 0
                if ($isAnc) {
        Write-Host "⚠️ Local '$SourceBranch' is behind remote; fast-forwarding..." -ForegroundColor Yellow
                    if ($route -eq "Route A") {
                        $cHead = (Invoke-Git @("symbolic-ref", "--short", "-q", "HEAD")).Output
                        if ($cHead -eq $SourceBranch) {
                            Invoke-Git @("merge", "--ff-only", "origin/$SourceBranch") | Out-Null
                        } else {
                            Invoke-Git @("update-ref", "refs/heads/$SourceBranch", $rSha) | Out-Null
                        }
                    } else {
                        Invoke-Git @("-C", $occupiedWorktreePath, "merge", "--ff-only", "origin/$SourceBranch") | Out-Null
                    }
                } elseif (-not $isRAnc) {
        throw "Source branch '$SourceBranch' has diverged from remote! Resolve conflict first."
                }
            }
        }
    }

    # Analyze net contributions
    $cherryRes = Invoke-Git @("cherry", "-v", $IntegrationBranch, $SourceBranch)
    Assert-GitSuccess $cherryRes "Execute git cherry analysis"
    $netCommits = [System.Collections.Generic.List[PSCustomObject]]::new()
    if (-not [string]::IsNullOrWhiteSpace($cherryRes.Output)) {
        foreach ($cl in ($cherryRes.Output -split "`r?`n")) {
            if ($cl -match '^\+\s+([0-9a-fA-F]+)\s+(.*)$') {
                $netCommits.Add([PSCustomObject]@{
                    Hash    = $Matches[1]
                    Subject = $Matches[2]
                })
            }
        }
    }

    $integCommit = (Invoke-Git @("rev-parse", "--short", $IntegrationBranch)).Output
    $sourceCommit = (Invoke-Git @("rev-parse", "--short", $SourceBranch)).Output
    $fullIntegCommit = (Invoke-Git @("rev-parse", $IntegrationBranch)).Output
    $fullSourceCommit = (Invoke-Git @("rev-parse", $SourceBranch)).Output

    if (-not $Apply) {
        Write-Host ""
    Write-Host "================== Branch Sync Single-Branch Pre-Flight Report ==================" -ForegroundColor Cyan
    Write-Host "Integration branch: $IntegrationBranch ($integCommit)"
    Write-Host "Source branch:      $SourceBranch ($sourceCommit)"
    Write-Host "Sync route:         $route $(if ($route -eq 'Route B') { "[$occupiedWorktreePath]" })"
    Write-Host "Net commits:        $($netCommits.Count) commits"
        foreach ($nc in $netCommits) {
            Write-Host "  + $($nc.Hash.Substring(0, [Math]::Min(7, $nc.Hash.Length))) $($nc.Subject)" -ForegroundColor White
        }
        Write-Host "----------------------------------------------------------------"
    Write-Host "Run command: pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -SourceBranch '$SourceBranch' -Apply" -ForegroundColor Yellow
        Write-Host "================================================================" -ForegroundColor Cyan
        Write-Host ""
        return
    }

    # Execution phase
    $timestamp = (Get-Date).ToString("yyyyMMdd-HHmmss")
    $cleanBranchTag = $SourceBranch -replace '[^a-zA-Z0-9_\-]', '_'
    $backupSourceRef = "refs/sync-backup/$cleanBranchTag/$timestamp-$sourceCommit"
    $backupIntegRef = "refs/sync-backup/$IntegrationBranch/$timestamp-$integCommit"
    Invoke-Git @("update-ref", $backupSourceRef, $fullSourceCommit) | Out-Null
    Invoke-Git @("update-ref", $backupIntegRef, $fullIntegCommit) | Out-Null
    Write-Host "🛡️ Safety backup created: $backupSourceRef" -ForegroundColor DarkCyan

    $initialBranch = (Invoke-Git @("rev-parse", "--abbrev-ref", "HEAD")).Output

    if ($netCommits.Count -eq 0) {
        Write-Host "Notice: Source branch has no net new commits, aligning directly..." -ForegroundColor Yellow
        if ($route -eq "Route A") {
            Invoke-Git @("checkout", $SourceBranch) | Out-Null
            Invoke-Git @("reset", "--hard", $IntegrationBranch) | Out-Null
            if ($doPush) { Invoke-Git @("push", "--force-with-lease", "origin", $SourceBranch) | Out-Null }
            Invoke-Git @("checkout", $IntegrationBranch) | Out-Null
        } else {
            Invoke-Git @("-C", $occupiedWorktreePath, "reset", "--hard", $IntegrationBranch) | Out-Null
            if ($doPush) { Invoke-Git @("-C", $occupiedWorktreePath, "push", "--force-with-lease", "origin", $SourceBranch) | Out-Null }
        }
    } else {
        if ($route -eq "Route A") {
            Invoke-Git @("checkout", $SourceBranch) | Out-Null
            $rb = Invoke-Git @("rebase", $IntegrationBranch)
            if ($rb.ExitCode -ne 0) {
                Invoke-Git @("rebase", "--abort") | Out-Null
                Invoke-Git @("checkout", $initialBranch) | Out-Null
            throw "Conflict encountered while rebasing '$SourceBranch'. State restored to backup: $backupSourceRef"
            }
            Invoke-Git @("checkout", $IntegrationBranch) | Out-Null
            Invoke-Git @("merge", "--ff-only", $SourceBranch) | Out-Null
        } else {
            Invoke-Git @("checkout", $IntegrationBranch) | Out-Null
            $hashes = $netCommits | ForEach-Object { $_.Hash }
            $cp = Invoke-Git (@("cherry-pick") + $hashes)
            if ($cp.ExitCode -ne 0) {
                Invoke-Git @("cherry-pick", "--abort") | Out-Null
                Invoke-Git @("reset", "--hard", $backupIntegRef) | Out-Null
                Invoke-Git @("checkout", $initialBranch) | Out-Null
            throw "Conflict occurred while cherry-picking net commits. Integration branch rolled back to pre-merge state ($backupIntegRef)."
            }
        }

    # Tree-Diff Guard v2 (cherry residue + file-level content containment)
        $guardRes = Test-BranchFullyMerged $IntegrationBranch $backupSourceRef
        if (-not $guardRes.Merged) {
            foreach ($mc in $guardRes.MissingCommits) {
        Write-Host "❌ Tree-Diff Guard: Source branch still has unapplied net commit: $mc" -ForegroundColor Red
            }
            foreach ($mf in $guardRes.MissingFiles) {
        Write-Host "❌ Tree-Diff Guard: Changes to file '$mf' not fully incorporated into integration branch!" -ForegroundColor Red
            }
            Invoke-Git @("reset", "--hard", $backupIntegRef) | Out-Null
            Invoke-Git @("checkout", $initialBranch) | Out-Null
        throw "Tree-Diff Guard blocked: source branch has incomplete commits/files; aborted and rolled back."
        }
    Write-Host "✅ Tree-Diff Guard audit passed: source branch changes 100% incorporated!" -ForegroundColor Green

        if ($doPush) {
            Invoke-Git @("push", "origin", $IntegrationBranch) | Out-Null
        }

    # Align source branch
        if ($route -eq "Route A") {
            Invoke-Git @("checkout", $SourceBranch) | Out-Null
            Invoke-Git @("reset", "--hard", $IntegrationBranch) | Out-Null
            if ($doPush) { Invoke-Git @("push", "--force-with-lease", "origin", $SourceBranch) | Out-Null }
            Invoke-Git @("checkout", $IntegrationBranch) | Out-Null
        } else {
            Invoke-Git @("-C", $occupiedWorktreePath, "reset", "--hard", $IntegrationBranch) | Out-Null
            if ($doPush) { Invoke-Git @("-C", $occupiedWorktreePath, "push", "--force-with-lease", "origin", $SourceBranch) | Out-Null }
        }
    }

    $allCompletedBranches = @($SourceBranch)

} else {
    # -----------------------------
# All-Branches Sync Mode (Default)
    # -----------------------------
Write-Host "🌐 Entering all-branches sync mode (target integration branch: '$IntegrationBranch')..." -ForegroundColor Cyan

# Scan all local branches and remote truth branches (candidates = local branches UNION remote truth)
# Must use %(refname) exact match on refs/heads/, avoiding %(refname:short) disambiguation prefixes (e.g. heads/origin)
    $localBranchRefs = (Invoke-Git @("for-each-ref", "--format=%(refname)", "refs/heads/")).Output -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $localBranches = [System.Collections.Generic.List[string]]::new()
    foreach ($lRef in $localBranchRefs) {
        if ($lRef.StartsWith("refs/heads/")) {
            $rawB = $lRef.Substring(11).Trim()
            $cleanB = Get-CleanBranchName $rawB $remoteNames
            if (-not [string]::IsNullOrWhiteSpace($cleanB)) {
                $localBranches.Add($cleanB)
            } else {
            Write-Warning "⚠️ Malformed local branch ref '$lRef' detected; safely ignored! Recommended to run cleanup."
            }
        }
    }

    $remoteBranchRefs = (Invoke-Git @("for-each-ref", "--format=%(refname)", "refs/remotes/origin/")).Output -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    $remoteBranches = [System.Collections.Generic.List[string]]::new()
    foreach ($rRef in $remoteBranchRefs) {
        if ($rRef.StartsWith("refs/remotes/origin/")) {
            $rawB = $rRef.Substring("refs/remotes/origin/".Length).Trim()
            if ($rawB -ne "HEAD") {
                $cleanB = Get-CleanBranchName $rawB $remoteNames
                if (-not [string]::IsNullOrWhiteSpace($cleanB)) {
                    $remoteBranches.Add($cleanB)
                } else {
            Write-Warning "⚠️ Malformed remote tracking ref '$rRef' detected; safely ignored!"
                }
            }
        }
    }

    # Remote truth discovery: trust only git ls-remote (unaffected by sandbox ref virtualization; see REFERENCE I).
    $remoteTruth = $null
    if ($hasRemote) {
        $remoteTruth = Get-RemoteTruthMap $remoteNames
    }

    $candidateBranchSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($lb in $localBranches) {
        if ($lb -ne $IntegrationBranch -and (Test-ValidBranchName $lb $remoteNames)) {
            $candidateBranchSet.Add($lb) | Out-Null
        }
    }
    if ($null -ne $remoteTruth) {
        foreach ($truthName in ($remoteTruth.Keys | Sort-Object)) {
            if ($truthName -eq $IntegrationBranch) { continue }
            if (-not (Test-ValidBranchName $truthName $remoteNames)) { continue }
            if ($candidateBranchSet.Contains($truthName)) { continue }
    # Exists on remote but missing locally: fetch by explicit SHA then create local tracking branch
            $truthSha = $remoteTruth[$truthName]
            Invoke-Git @("fetch", "origin", $truthSha) | Out-Null
            $trackRes = Invoke-Git @("branch", "--track", $truthName, $truthSha)
            if ($trackRes.ExitCode -ne 0) {
                $refRes = Invoke-Git @("update-ref", "refs/heads/$truthName", $truthSha)
                if ($refRes.ExitCode -ne 0) {
                    $msg = if ($refRes.Error) { $refRes.Error } else { $refRes.Output }
                throw "Failed to create local tracking ref for remote branch '$truthName': $msg"
                }
            }
            $verifyRes = Invoke-Git @("rev-parse", "--verify", $truthName)
            if ($verifyRes.ExitCode -ne 0) {
                throw "Failed to create local tracking ref for remote branch '$truthName': local ref still not visible after creation."
            }
            $candidateBranchSet.Add($truthName) | Out-Null
        }
    } else {
        if ($hasRemote) {
    Write-Host "⚠️ Unable to fetch remote truth (git ls-remote failed, possibly offline); falling back to local refs/remotes/origin enumeration." -ForegroundColor Yellow
        }
        foreach ($shortB in $remoteBranches) {
            if ($shortB -ne $IntegrationBranch -and (Test-ValidBranchName $shortB $remoteNames)) {
                if (-not $candidateBranchSet.Contains($shortB)) {
        # Missing locally: create local tracking branch
                    Invoke-Git @("branch", "--track", $shortB, "origin/$shortB") | Out-Null
                    $candidateBranchSet.Add($shortB) | Out-Null
                }
            }
        }
    }

# Quarantined diverged branches: branches diverged from remote truth are never reset, pushed, or merged
    $divergedBranches = [System.Collections.Generic.List[string]]::new()

# Dual-end alignment: evaluate fast-forward/ahead/diverged based on remote truth (ls-remote)
    if ($hasRemote -and (-not $NoFetch)) {
        foreach ($b in @($candidateBranchSet)) {
            $truthSha = $null
            $hasTruth = $false
            if (($null -ne $remoteTruth) -and $remoteTruth.ContainsKey($b)) {
                $truthSha = $remoteTruth[$b]
                $hasTruth = $true
            } elseif ((Invoke-Git @("rev-parse", "--verify", "origin/$b")).ExitCode -eq 0) {
                $truthSha = (Invoke-Git @("rev-parse", "origin/$b")).Output.Trim()
                $hasTruth = $true
            }
            if (-not $hasTruth) { continue }
            $lSha = (Invoke-Git @("rev-parse", $b)).Output.Trim()
            $rSha = $truthSha.Trim()
            if ($lSha -eq $rSha) { continue }
            $isAnc = (Invoke-Git @("merge-base", "--is-ancestor", $lSha, $rSha)).ExitCode -eq 0
            $isRAnc = (Invoke-Git @("merge-base", "--is-ancestor", $rSha, $lSha)).ExitCode -eq 0
            if ($isAnc) {
                if ($wtInfo.BranchToPath.ContainsKey($b)) {
                    Invoke-Git @("-C", $wtInfo.BranchToPath[$b], "merge", "--ff-only", "origin/$b") | Out-Null
                } else {
                    Invoke-Git @("update-ref", "refs/heads/$b", $rSha) | Out-Null
                }
            } elseif ($isRAnc) {
            # Local ahead of remote: local has unique commits, keep untouched (handled during merge phase)
                continue
            } else {
                $divergedBranches.Add($b)
            Write-Host "❌ Branch '$b' has diverged from remote and is quarantined: will not be reset, pushed, or merged!" -ForegroundColor Red
            Write-Host "   Manual resolution (see REFERENCE III):" -ForegroundColor Yellow
        Write-Host "  1. git cherry -v $IntegrationBranch origin/<branch>  # '+' lines are remote net commits" -ForegroundColor White
        Write-Host "  2. In integration branch checkout, cherry-pick these net commits in order, then git push origin $IntegrationBranch" -ForegroundColor White
        Write-Host "  3. In the branch worktree, git reset --hard <integration branch tip>" -ForegroundColor White
        Write-Host "  4. Push explicitly using old remote sha as lease value" -ForegroundColor White
                $candidateBranchSet.Remove($b) | Out-Null
            }
        }
    }

# Collect net contributions from all branches and sort chronologically by committerdate
# Net contributions = unreachable set: git rev-list <branch> --not <IntegrationBranch>
    # Empty commits have no patch-id (return empty string); callers handle empty commits independently.
$allNetCommitsMap = @{} # PatchKey -> CommitInfo (globally unique)
$patchProvenance = @{}  # PatchKey -> List[string] (branches sharing this patch-id)
$branchNetMap = @{}     # Branch -> List of CommitInfo (per-branch reporting)

    foreach ($b in ($candidateBranchSet | Sort-Object)) {
        $revRes = Invoke-Git @("rev-list", $b, "--not", $IntegrationBranch, "--reverse")
        $bNetList = [System.Collections.Generic.List[PSCustomObject]]::new()
        if ($revRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($revRes.Output))) {
            foreach ($h in ($revRes.Output -split "`r?`n")) {
                if ([string]::IsNullOrWhiteSpace($h)) { continue }
                $fullHash = $h.Trim()

        # Retrieve detailed metadata for chronological sorting, deduplication, and provenance tracking
                $logInfo = Invoke-Git @("log", "-1", "--format=%H|%ct|%s", $fullHash)
                $committerTime = [long]0
                $cSubj = ""
                if ($logInfo.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($logInfo.Output))) {
                    $parts = $logInfo.Output.Split('|')
                    $fullHash = $parts[0].Trim()
                    if ($parts.Length -gt 1) { [long]::TryParse($parts[1].Trim(), [ref]$committerTime) | Out-Null }
                    if ($parts.Length -gt 2) { $cSubj = $parts[2].Trim() }
                }
                $shortHash = $fullHash.Substring(0, [Math]::Min(7, $fullHash.Length))

    # Patch context drift tolerance: if identical commit with preserved content exists on integration branch,
                if (-not [string]::IsNullOrWhiteSpace($cSubj)) {
                    $landedBySubj = Invoke-Git @("log", $IntegrationBranch, "--fixed-strings", "--grep=$cSubj", "-n", "5", "--format=%H")
                    if ($landedBySubj.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($landedBySubj.Output))) {
                        $matched = $false
                        foreach ($candHash in ($landedBySubj.Output -split "`r?`n")) {
                            $ch = $candHash.Trim()
                            if (-not [string]::IsNullOrWhiteSpace($ch) -and (Test-ContentPreserved $fullHash $ch)) {
                                $matched = $true
                                break
                            }
                        }
                        if ($matched) {
                            continue
                        }
                    }
                }

                $patchId = Get-CommitPatchId $fullHash
                $dedupKey = if (-not [string]::IsNullOrWhiteSpace($patchId)) { $patchId } else { $fullHash }

                $cObj = [PSCustomObject]@{
                    Hash          = $shortHash
                    FullHash      = $fullHash
                    Subject       = $cSubj
                    CommitterTime = $committerTime
                    PatchId       = $patchId
                    SourceBranch  = $b
                }
                $bNetList.Add($cObj)

        # Global deduplication (identical patch-id = same change), preserving source branch list
                if (-not $patchProvenance.ContainsKey($dedupKey)) {
                    $patchProvenance[$dedupKey] = [System.Collections.Generic.List[string]]::new()
                }
                $patchProvenance[$dedupKey].Add($b)
                if (-not $allNetCommitsMap.ContainsKey($dedupKey)) {
                    $allNetCommitsMap[$dedupKey] = $cObj
                }
            }
        }
        $branchNetMap[$b] = $bNetList
    }

# Sort globally in ascending order by timestamp
    $orderedNetCommits = $allNetCommitsMap.Values | Sort-Object -Property CommitterTime, FullHash
    $branchesWithNet = @($branchNetMap.Keys | Where-Object { $branchNetMap[$_].Count -gt 0 })

    $integCommit = (Invoke-Git @("rev-parse", "--short", $IntegrationBranch)).Output
    $fullIntegCommit = (Invoke-Git @("rev-parse", $IntegrationBranch)).Output

# Pre-flight report
    if (-not $Apply) {
        Write-Host ""
Write-Host "================== All-Branches Sync Pre-Flight Topology (Dry Run) ==================" -ForegroundColor Cyan
    Write-Host "Integration branch: $IntegrationBranch ($integCommit)"
Write-Host "Total branches scanned:     $($candidateBranchSet.Count)"
Write-Host "Branches with net commits:  $($branchesWithNet.Count) ($(($branchesWithNet | ForEach-Object { "'$_'" }) -join ', '))"
Write-Host "Unique net commits to merge:$($orderedNetCommits.Count) (chronologically sorted)"
        Write-Host "------------------------------------------------------------------"
        if ($orderedNetCommits.Count -gt 0) {
Write-Host "Global Chronological Commit Order:" -ForegroundColor DarkCyan
            $idx = 1
            foreach ($oc in $orderedNetCommits) {
                Write-Host "  [$idx] $($oc.Hash.Substring(0, [Math]::Min(7, $oc.Hash.Length))) $($oc.Subject) (from: $($oc.SourceBranch))" -ForegroundColor White
                $idx++
            }
        } else {
    Write-Host "✅ All branches are already aligned with integration branch; no new commits to merge." -ForegroundColor Green
        }
        Write-Host "------------------------------------------------------------------"
        if ($divergedBranches.Count -gt 0) {
    Write-Host "Quarantined diverged branches: $($divergedBranches.Count) ($(($divergedBranches | ForEach-Object { "'$_'" }) -join ', ')); local/remote divergence conflict (see REFERENCE III)" -ForegroundColor Red
    Write-Host "⚠️ Warning: Diverged branches detected! -Apply will block execution to prevent partial merges or resets!" -ForegroundColor Red
            Write-Host "------------------------------------------------------------------"
        }
    Write-Host "To execute sync, alignment, and push for all branches:" -ForegroundColor Yellow
        Write-Host "  pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -Apply" -ForegroundColor White
        Write-Host "==================================================================" -ForegroundColor Cyan
        Write-Host ""
        return
    }

    # Execution phase
    if ($divergedBranches.Count -gt 0) {
    Write-Host "❌ Severe block: The following branches are quarantined as diverged: $($divergedBranches -join ', ')!" -ForegroundColor Red
    Write-Host "   Diverged branches have conflicting local and remote commits; proceeding could lead to code loss or overwrites." -ForegroundColor Yellow
    Write-Host "   Please resolve diverged branches manually following instructions above before running -Apply!" -ForegroundColor Yellow
    throw "Execution blocked: Diverged branches $($divergedBranches -join ', ') detected; blocked to prevent code loss!"
    }

    $timestamp = (Get-Date).ToString("yyyyMMdd-HHmmss")
    $backupIntegRef = "refs/sync-backup/$IntegrationBranch/$timestamp-$integCommit"
    Invoke-Git @("update-ref", $backupIntegRef, $fullIntegCommit) | Out-Null
    Write-Host "🛡️ Integration branch safety backup created: $backupIntegRef" -ForegroundColor DarkCyan

    $backupSourceRefs = @{}
    foreach ($b in $candidateBranchSet) {
        $bSha = (Invoke-Git @("rev-parse", $b)).Output
        $bTag = $b -replace '[^a-zA-Z0-9_\-]', '_'
        $bRef = "refs/sync-backup/$bTag/$timestamp-$($bSha.Substring(0, [Math]::Min(7, $bSha.Length)))"
        Invoke-Git @("update-ref", $bRef, $bSha) | Out-Null
        $backupSourceRefs[$b] = $bRef
    }

    # Ensure integration branch is currently checked out
    $cHead = (Invoke-Git @("symbolic-ref", "--short", "-q", "HEAD")).Output
    if ($cHead -ne $IntegrationBranch) {
        $coInteg = Invoke-Git @("checkout", $IntegrationBranch)
    Assert-GitSuccess $coInteg "Checkout integration branch $IntegrationBranch"
    }

    # Cherry-pick chronologically sorted net commits in sequence
    if ($orderedNetCommits.Count -gt 0) {
    Write-Host ">> Starting chronological cherry-pick of $($orderedNetCommits.Count) net commits..." -ForegroundColor Cyan
        $gitDir = (Invoke-Git @("rev-parse", "--git-dir")).Output
        try { $gitDir = (Resolve-Path $gitDir).Path } catch {}
        $stateFilePath = Join-Path $gitDir "branch-sync-state.json"

        $editorCmd = if ([System.Environment]::OSVersion.Platform -like "*Win*" -or $IsWindows) { "cmd.exe /c exit 0" } else { "true" }
        $currentPickIdx = 0
        $skippedEmpty = [System.Collections.Generic.List[PSCustomObject]]::new()
        foreach ($oc in $orderedNetCommits) {
            Write-Host "  [$($currentPickIdx + 1)/$($orderedNetCommits.Count)] Cherry-picking $($oc.Hash.Substring(0, [Math]::Min(7, $oc.Hash.Length))) $($oc.Subject)..." -ForegroundColor DarkGray
            $cp = Invoke-Git @("-c", "core.editor=$editorCmd", "cherry-pick", $oc.FullHash)
            if ($cp.ExitCode -ne 0) {
            # Empty commit / duplicate commit: no unmerged files means equivalently landed; skip rather than interrupt
                $unmergedCheck = Invoke-Git @("diff", "--name-only", "--diff-filter=U")
                $hasUnmerged = (($unmergedCheck.ExitCode -eq 0) -and (-not [string]::IsNullOrWhiteSpace($unmergedCheck.Output)))
                if (-not $hasUnmerged) {
                    Invoke-Git @("cherry-pick", "--skip") | Out-Null
                    $skippedEmpty.Add($oc)
            Write-Host "  ⏭️ Empty commit skipped (changes already exist): $($oc.FullHash.Substring(0, [Math]::Min(7, $oc.FullHash.Length))) $($oc.Subject)" -ForegroundColor Yellow
                    $currentPickIdx++
                    continue
                }
            # True conflict! Persist current sync state
                $stateObj = @{
                    IntegrationBranch  = $IntegrationBranch
                    CurrentCommitIndex = $currentPickIdx
                    OrderedCommits     = $orderedNetCommits
                    BackupIntegRef     = $backupIntegRef
                    BackupSourceRefs   = $backupSourceRefs
                    AllSourceBranches  = @($candidateBranchSet)
                    DeclaredVerifyCmd  = $declaredVerifyCmd
                    NoPush             = [bool]$NoPush
                }
                [System.IO.File]::WriteAllText($stateFilePath, (ConvertTo-Json $stateObj -Depth 10), [System.Text.Encoding]::UTF8)

                $unmerged = (Invoke-Git @("diff", "--name-only", "--diff-filter=U")).Output
                Write-Host ""
            Write-Host "================== ⚠️ CHERRY-PICK CONFLICT ⚠️ ==================" -ForegroundColor Yellow
            Write-Host "Conflicted commit: $($oc.Hash) - $($oc.Subject)"
            Write-Host "Conflicted files list:" -ForegroundColor Red
                foreach ($uf in ($unmerged -split "`r?`n")) {
                    Write-Host "  ! $uf" -ForegroundColor Red
                }
                Write-Host "------------------------------------------------------------------"
            Write-Host "Resolution guide:" -ForegroundColor Cyan
            Write-Host "  1. Resolve conflict markers (<<<<<<< / ======= / >>>>>>>) in the files above;"
            Write-Host "  2. Run: git add <resolved files>"
            Write-Host "  3. Run 1-Shot recovery script to continue remaining commits and auto-align:" -ForegroundColor Green
                Write-Host "     pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Continue" -ForegroundColor White
            Write-Host "  4. To abort and restore pre-sync state:" -ForegroundColor DarkGray
                Write-Host "     pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Abort" -ForegroundColor White
                Write-Host "======================================================================" -ForegroundColor Yellow
                Write-Host ""
                exit 1
            }
        # Per-commit content preservation check: landed commit must preserve diff lines
            $landedHash = (Invoke-Git @("rev-parse", "HEAD")).Output.Trim()
            if (-not (Test-ContentPreserved $oc.FullHash $landedHash)) {
            throw "Content preservation check failed: commit $($oc.FullHash) ($($oc.Subject)) was incomplete after landing; aborted. Roll back via safety backup $backupIntegRef."
            }
            $currentPickIdx++
        }
    Write-Host "✅ All $($orderedNetCommits.Count) commits successfully linearly applied to integration branch!" -ForegroundColor Green
        if ($skippedEmpty.Count -gt 0) {
        Write-Host "⏭️ $($skippedEmpty.Count) empty/duplicate commits skipped (changes already exist):" -ForegroundColor Yellow
            foreach ($sk in $skippedEmpty) {
                Write-Host "  ⏭️ $($sk.FullHash.Substring(0, [Math]::Min(7, $sk.FullHash.Length))) $($sk.Subject) (from: $($sk.SourceBranch))" -ForegroundColor Yellow
            }
        }
    }

    # Tree-Diff Guard v2 audit (cherry residue + file-level content containment)
    Write-Host "🔍 Executing post-merge change preservation audit (Tree-Diff Guard v2)..." -ForegroundColor DarkCyan
    foreach ($b in $branchesWithNet) {
        $bRef = $backupSourceRefs[$b]
        $guardRes = Test-BranchFullyMerged $IntegrationBranch $bRef
        if (-not $guardRes.Merged) {
            foreach ($mc in $guardRes.MissingCommits) {
                Write-Host "❌ Severe block: Source branch '$b' has unapplied net commit: $mc!" -ForegroundColor Red
            }
            foreach ($mf in $guardRes.MissingFiles) {
                Write-Host "❌ Severe block: Changes to file '$mf' from source branch '$b' not fully incorporated!" -ForegroundColor Red
            }
            Invoke-Git @("reset", "--hard", $backupIntegRef) | Out-Null
            throw "Tree-Diff Guard v2 blocked: unapplied commits/files detected; rolled back integration branch to pre-sync backup."
        }
    }
    Write-Host "✅ Tree-Diff Guard v2 audit passed: all branch changes 100% preserved!" -ForegroundColor Green

    # Push integration branch
    if ($doPush) {
        Write-Host ">> Pushing integration branch origin/$IntegrationBranch..." -ForegroundColor DarkGray
        $pInteg = Invoke-Git @("push", "origin", $IntegrationBranch)
        Assert-GitSuccess $pInteg "Pushing integration branch $IntegrationBranch"
    }

    # Batch align all branches and worktrees
    Write-Host ">> Batch aligning all branches and Worktrees to latest integration branch tip..." -ForegroundColor Cyan
    $integTip = (Invoke-Git @("rev-parse", $IntegrationBranch)).Output

    # Strictly verify cleanliness of all external worktrees before resetting
    foreach ($b in $candidateBranchSet) {
        if ($wtInfo.BranchToPath.ContainsKey($b)) {
            $wPath = $wtInfo.BranchToPath[$b]
            if ($wPath -ne $currentWorktreePath) {
                $wtDirty = Get-DirtyItems $wtInfo.AllPaths $declaredDirtyPatterns $wPath
                if ($wtDirty.Count -gt 0) {
                    throw "External Worktree ($wPath) occupying branch '$b' has uncommitted changes. Forced reset aborted:`n$($wtDirty -join "`n")"
                }
            }
        }
    }

    foreach ($b in $candidateBranchSet) {
        if ($wtInfo.BranchToPath.ContainsKey($b)) {
            $wPath = $wtInfo.BranchToPath[$b]
            if ($wPath -ne $currentWorktreePath) {
                Invoke-Git @("-C", $wPath, "reset", "--hard", $integTip) | Out-Null
            }
        } else {
            Invoke-Git @("branch", "-f", $b, $integTip) | Out-Null
        }
        Write-Host "  ✅ Branch '$b' aligned" -ForegroundColor Green
    }

    # Batch push all aligned branches
    if ($doPush) {
        Write-Host ">> Safely pushing all aligned branches (--force-with-lease)..." -ForegroundColor Cyan
        $pushBranches = @($candidateBranchSet | Where-Object { Test-ValidBranchName $_ $remoteNames })
        if ($pushBranches.Count -gt 0) {
            $pArgs = @("push", "--force-with-lease", "origin") + $pushBranches
            $pRes = Invoke-Git $pArgs
            if ($pRes.ExitCode -ne 0) {
                foreach ($pb in $pushBranches) {
                    Invoke-Git @("push", "--force-with-lease", "origin", $pb) | Out-Null
                }
            }
            Write-Host "  ✅ All $($pushBranches.Count) aligned branches safely pushed to origin" -ForegroundColor Green
        }
    }

    $allCompletedBranches = @($candidateBranchSet)

    # Quarantine final state: healthy branches pushed/aligned; diverged branches untouched, list and exit 1
    if ($divergedBranches.Count -gt 0) {
        Write-Host ""
        Write-Host "================== BRANCH SYNC QUARANTINED (Some Branches Quarantined) ==================" -ForegroundColor Yellow
        Write-Host "Healthy branches synced/aligned/pushed: $($allCompletedBranches.Count)" -ForegroundColor Green
        Write-Host "$($divergedBranches.Count) diverged branches were left untouched and require manual resolution:" -ForegroundColor Red
        foreach ($db in $divergedBranches) {
            Write-Host "  ❌ $db" -ForegroundColor Red
        }
        Write-Host "Manual resolution (see REFERENCE III, per branch):" -ForegroundColor Yellow
        Write-Host "  1. git cherry -v $IntegrationBranch origin/<branch>  # '+' lines are remote net commits" -ForegroundColor White
        Write-Host "  2. In integration branch checkout, cherry-pick these net commits in order, then git push origin $IntegrationBranch" -ForegroundColor White
        Write-Host "  3. In the branch worktree, git reset --hard <integration branch tip>" -ForegroundColor White
        Write-Host "  4. Push explicitly using old remote sha as lease value" -ForegroundColor White
        Write-Host "=================================================================================" -ForegroundColor Yellow
        Write-Host ""
        exit 1
    }
}

# 6. Project post-merge verification command (1-Shot closed loop)
$verifyStatus = "SKIPPED"
if (-not $NoVerify -and (-not [string]::IsNullOrWhiteSpace($declaredVerifyCmd))) {
    Write-Host ""
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
} elseif ($NoVerify) {
    $verifyStatus = "SKIPPED (-NoVerify specified)"
} else {
    $verifyStatus = "NONE (No verification command registered in SKILL.md)"
}

# 7. Final delivery dashboard
$integTipFinal = (Invoke-Git @("rev-parse", $IntegrationBranch)).Output
Write-Host ""
Write-Host "================== BRANCH SYNC SUCCESS ==================" -ForegroundColor Green
Write-Host "Integration branch: $IntegrationBranch ($($integTipFinal.Substring(0, [Math]::Min(7, $integTipFinal.Length))))"
Write-Host "Synced branches:    $($allCompletedBranches.Count) branches (all aligned and safely pushed)"
Write-Host "Preservation audit: ✅ PASSED (Tree-Diff 100% preserved)"
Write-Host "Dual-end alignment: ✅ All aligned"
Write-Host "Gate verification:  $(if ($verifyStatus.StartsWith('PASSED')) { "✅ $verifyStatus" } elseif ($verifyStatus.StartsWith('FAILED')) { "❌ $verifyStatus" } else { "ℹ️ $verifyStatus" })"
Write-Host "Working tree state: ✅ Clean"
Write-Host "Final status:       COMPLETED_READY_TO_REPORT"
Write-Host "=========================================================" -ForegroundColor Green

if ($verifyStatus.StartsWith("FAILED")) {
    Write-Warning "Project verification gates did not pass. Inspect test output above and resolve!"
} else {
    Write-Host "Branch sync and post-merge verification completed successfully; ready to report to user." -ForegroundColor Cyan
}
Write-Host ""