<#
.SYNOPSIS
    Read-only branch topology and net contribution inspection tool (Show Branch Topology)

.DESCRIPTION
    Inspects all local branches, remote tracking states, worktree attachments, and net new commits relative to the integration branch (git cherry).
    Read-only; performs no write operations, designed for agents or developers to quickly assess branch state.

.PARAMETER IntegrationBranch
    Integration branch name. If omitted, automatically inferred from active branch in current directory or fallback strategies.

.PARAMETER Detailed
    Display commit subjects for net contributions.

.PARAMETER NoFetch
    Skip git fetch origin.
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$IntegrationBranch = "",

    [switch]$Detailed,
    [switch]$NoFetch
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

function Test-ContentPreserved {
    # Per-commit content preservation verification (see REFERENCE IV): compare file added/removed lines,
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
    $map = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    $allPaths = [System.Collections.Generic.List[string]]::new()
    $wt = Invoke-Git @("worktree", "list", "--porcelain")
    if ($wt.ExitCode -eq 0) {
        $lines = $wt.Output -split "`r?`n"
        $currentPath = ""
        foreach ($line in $lines) {
            if ($line.StartsWith("worktree ")) {
                $currentPath = $line.Substring(9).Trim()
                try { $currentPath = (Resolve-Path $currentPath).Path } catch {}
                if ($currentPath -and (-not $allPaths.Contains($currentPath))) {
                    $allPaths.Add($currentPath)
                }
            } elseif ($line.StartsWith("branch refs/heads/")) {
                $bName = $line.Substring(18).Trim()
                if ($bName -and $currentPath) {
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

# Read SKILL.md
$skillDocPath = Join-Path $PSScriptRoot "..\SKILL.md"
if (-not (Test-Path $skillDocPath)) {
    $candidate = Join-Path (Get-Location).Path ".agents\skills\git-workflow\SKILL.md"
    if (Test-Path $candidate) {
        $skillDocPath = $candidate
    }
}
$declaredIntegrationBranch = ""
if (Test-Path $skillDocPath) {
    try {
        $skillDocContent = [System.IO.File]::ReadAllText($skillDocPath, [System.Text.Encoding]::UTF8)
        if ($skillDocContent -match '(?m)^[-*]\s*(?:\u96c6\u6210\u5206\u652f|Integration branch)[\uFF1A:]\s*`?([a-zA-Z0-9_\-\.\/]+)`?') {
            $declaredIntegrationBranch = $Matches[1].Trim()
        }
    } catch {}
}

$hasRemote = ((Invoke-Git @("remote", "get-url", "origin")).ExitCode -eq 0)
$remoteNames = Get-RemoteNames
$IntegrationBranch = Resolve-IntegrationBranch $IntegrationBranch $declaredIntegrationBranch $hasRemote $remoteNames

$integCheck = Invoke-Git @("rev-parse", "--verify", $IntegrationBranch)
if ($integCheck.ExitCode -ne 0) {
    throw "Integration branch '$IntegrationBranch' does not exist."
}
$integShaShort = (Invoke-Git @("rev-parse", "--short", $IntegrationBranch)).Output

if ($hasRemote -and (-not $NoFetch)) {
    Write-Host "Fetching latest remote refs (git fetch --prune)..." -ForegroundColor DarkGray
    Invoke-Git @("fetch", "--prune", "origin") | Out-Null
}

$wtInfo = Get-WorktreeMap
$currentDir = (Get-Location).Path
try { $currentDir = (Resolve-Path $currentDir).Path } catch {}

# Get all local branches
$localBranchRaw = Invoke-Git @("for-each-ref", "--format=%(refname)|%(objectname:short)|%(upstream:short)|%(upstream:track)", "refs/heads/")
$localBranches = [System.Collections.Generic.Dictionary[string, PSCustomObject]]::new([System.StringComparer]::OrdinalIgnoreCase)

if ($localBranchRaw.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($localBranchRaw.Output))) {
    foreach ($line in ($localBranchRaw.Output -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $parts = $line.Split('|')
        $ref = $parts[0].Trim()
        $sha = if ($parts.Length -gt 1) { $parts[1].Trim() } else { "" }
        $upstream = if ($parts.Length -gt 2) { $parts[2].Trim() } else { "" }
        $track = if ($parts.Length -gt 3) { $parts[3].Trim() } else { "" }

        if ($ref.StartsWith("refs/heads/")) {
            $b = $ref.Substring(11).Trim()
            $cleanB = Get-CleanBranchName $b $remoteNames
            if (-not [string]::IsNullOrWhiteSpace($cleanB)) {
                $localBranches[$cleanB] = [PSCustomObject]@{
                    Sha      = $sha
                    Upstream = $upstream
                    Track    = $track
                }
            }
        }
    }
}

# Get all remote branches
$remoteBranches = [System.Collections.Generic.Dictionary[string, PSCustomObject]]::new([System.StringComparer]::OrdinalIgnoreCase)
if ($hasRemote) {
    $remoteBranchRaw = Invoke-Git @("for-each-ref", "--format=%(refname)|%(objectname:short)", "refs/remotes/origin/")
    if ($remoteBranchRaw.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($remoteBranchRaw.Output))) {
        foreach ($line in ($remoteBranchRaw.Output -split "`r?`n")) {
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            $parts = $line.Split('|')
            $ref = $parts[0].Trim()
            $sha = if ($parts.Length -gt 1) { $parts[1].Trim() } else { "" }

            if ($ref.StartsWith("refs/remotes/origin/")) {
                $b = $ref.Substring("refs/remotes/origin/".Length).Trim()
                if ($b -eq "HEAD") { continue }
                $cleanB = Get-CleanBranchName $b $remoteNames
                if (-not [string]::IsNullOrWhiteSpace($cleanB)) {
                    $remoteBranches[$cleanB] = [PSCustomObject]@{
                        Sha = $sha
                    }
                }
            }
        }
    }
}

# Aggregate all branch names
$allBranchNames = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($k in $localBranches.Keys) { $allBranchNames.Add($k) | Out-Null }
foreach ($k in $remoteBranches.Keys) { $allBranchNames.Add($k) | Out-Null }

Write-Host ""
Write-Host "========================== Branch Topology & Net Contribution ==========================" -ForegroundColor Cyan
Write-Host "Integration branch: $IntegrationBranch ($integShaShort)" -ForegroundColor Green
Write-Host "Current directory:  $currentDir" -ForegroundColor DarkGray
Write-Host "----------------------------------------------------------------------------------------"

$branchReportList = [System.Collections.Generic.List[PSCustomObject]]::new()
$totalPendingCommits = 0
$branchesWithNetCommits = 0

foreach ($bName in ($allBranchNames | Sort-Object)) {
    $hasLocal = $localBranches.ContainsKey($bName)
    $hasRemoteB = $remoteBranches.ContainsKey($bName)
    $lSha = if ($hasLocal) { $localBranches[$bName].Sha } else { "-" }
    $rSha = if ($hasRemoteB) { $remoteBranches[$bName].Sha } else { "-" }

    # Determine tracking status
    $remoteStatus = "LocalOnly"
    if ($hasLocal -and $hasRemoteB) {
        if ($lSha -eq $rSha) {
            $remoteStatus = "Synced"
        } else {
            $isAnc = (Invoke-Git @("merge-base", "--is-ancestor", $lSha, $rSha)).ExitCode -eq 0
            $isRAnc = (Invoke-Git @("merge-base", "--is-ancestor", $rSha, $lSha)).ExitCode -eq 0
            if ($isAnc) {
                $aheadCount = (Invoke-Git @("rev-list", "--count", "$lSha..$rSha")).Output
                $remoteStatus = "Behind ($aheadCount)"
            } elseif ($isRAnc) {
                $behindCount = (Invoke-Git @("rev-list", "--count", "$rSha..$lSha")).Output
                $remoteStatus = "Ahead ($behindCount)"
            } else {
                $remoteStatus = "Diverged"
            }
        }
    } elseif ($hasRemoteB -and (-not $hasLocal)) {
        $remoteStatus = "RemoteOnly"
    }

    # Determine Worktree status
    $wtStatus = "-"
    if ($wtInfo.BranchToPath.ContainsKey($bName)) {
        $p = $wtInfo.BranchToPath[$bName]
        if ($p -eq $currentDir) {
            $wtStatus = "Current"
        } else {
            $wtStatus = "Worktree"
        }
    }

    # Determine net contributions (git cherry -v against IntegrationBranch)
    $netCount = 0
    $netList = [System.Collections.Generic.List[string]]::new()
    $evalRef = if ($hasLocal) { $bName } else { "origin/$bName" }
    if ($bName -ne $IntegrationBranch) {
        $cherryRes = Invoke-Git @("cherry", "-v", $IntegrationBranch, $evalRef)
        if ($cherryRes.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($cherryRes.Output))) {
            foreach ($cline in ($cherryRes.Output -split "`r?`n")) {
                if ($cline -match '^\+\s+([0-9a-fA-F]+)\s+(.*)$') {
                    $resHash = $Matches[1]
                    $cSubj = $Matches[2].Trim()
                    # Patch context drift tolerance: if identical commit with preserved content exists on integration branch, treat as merged
                    if (-not [string]::IsNullOrWhiteSpace($cSubj)) {
                        $landedBySubj = Invoke-Git @("log", $IntegrationBranch, "--fixed-strings", "--grep=$cSubj", "-n", "5", "--format=%H")
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
                    $netCount++
                    $netList.Add("$($resHash.Substring(0, [Math]::Min(7, $resHash.Length))) $cSubj")
                }
            }
        }
    }

    if ($netCount -gt 0) {
        $branchesWithNetCommits++
        $totalPendingCommits += $netCount
    }

    $branchReportList.Add([PSCustomObject]@{
        Branch       = $bName
        IsInteg      = ($bName -eq $IntegrationBranch)
        LocalSha     = $lSha
        RemoteStatus = $remoteStatus
        NetCommits   = $netCount
        NetList      = $netList
        Worktree     = $wtStatus
    })
}

# Format table output
$fmt = "{0,-24} {1,-10} {2,-16} {3,-12} {4,-10}"
Write-Host ([string]::Format($fmt, "Branch Name", "Local SHA", "Remote Status", "Net Commits", "Worktree")) -ForegroundColor DarkGray
Write-Host ("-" * 74) -ForegroundColor DarkGray

foreach ($row in $branchReportList) {
    $netStr = if ($row.IsInteg) { "(Integration)" } elseif ($row.NetCommits -eq 0) { "0 (Aligned)" } else { "+$($row.NetCommits) (Pending)" }
    $lineStr = [string]::Format($fmt, $row.Branch, $row.LocalSha, $row.RemoteStatus, $netStr, $row.Worktree)
    
    if ($row.IsInteg) {
        Write-Host $lineStr -ForegroundColor Cyan
    } elseif ($row.NetCommits -gt 0) {
        Write-Host $lineStr -ForegroundColor Yellow
    } elseif ($row.RemoteStatus -eq "Diverged") {
        Write-Host $lineStr -ForegroundColor Red
    } else {
        Write-Host $lineStr -ForegroundColor White
    }

    if ($Detailed -and $row.NetList.Count -gt 0) {
        foreach ($nl in $row.NetList) {
            Write-Host "   + $nl" -ForegroundColor DarkYellow
        }
    }
}

Write-Host "----------------------------------------------------------------------------------------"
Write-Host "Summary: Scanned $($branchReportList.Count) branches | $branchesWithNetCommits branches with net commits | Total pending net commits: $totalPendingCommits" -ForegroundColor Cyan
if ($totalPendingCommits -gt 0) {
    Write-Host "⚡ Recommended 1-Shot command to sync all branches:" -ForegroundColor Yellow
    Write-Host "   pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -Apply" -ForegroundColor White
} else {
    Write-Host "✅ All branches are already included in the integration branch. Nothing to sync." -ForegroundColor Green
}
Write-Host "========================================================================================" -ForegroundColor Cyan
Write-Host ""