<#
.SYNOPSIS
    Branch sync safety backup management tool (Manage Sync Backups)

.DESCRIPTION
    Inspect, restore, or clean refs/sync-backup/* persistent backup refs created by branch-sync.

.PARAMETER List
    List all existing safety backups.

.PARAMETER RestoreBranch
    Target branch name to restore.

.PARAMETER BackupRef
    Backup ref to restore to (e.g. refs/sync-backup/dev/20261007-120000-xxxx).

.PARAMETER Clean
    Clean expired safety backups.

.PARAMETER OlderThanDays
    Clean backups older than specified days (default 7; 0 to clean all).
#>

[CmdletBinding()]
param(
    [switch]$List,
    [string]$RestoreBranch = "",
    [string]$BackupRef = "",
    [switch]$Clean,
    [int]$OlderThanDays = 7,
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

if (-not [string]::IsNullOrWhiteSpace($RestoreBranch) -and -not [string]::IsNullOrWhiteSpace($BackupRef)) {
    Write-Host ">> Restoring branch '$RestoreBranch' from safety backup..." -ForegroundColor Cyan
    $chk = Invoke-Git @("rev-parse", "--verify", $BackupRef)
    if ($chk.ExitCode -ne 0) {
        throw "Specified backup reference does not exist: $BackupRef"
    }
    $targetSha = $chk.Output

    # Check if occupied by a worktree
    $wtList = Invoke-Git @("worktree", "list", "--porcelain")
    $wtPath = $null
    if ($wtList.ExitCode -eq 0) {
        $lines = $wtList.Output -split "`r?`n"
        $tempP = ""
        foreach ($line in $lines) {
            if ($line.StartsWith("worktree ")) {
                $tempP = $line.Substring(9).Trim()
            } elseif ($line.StartsWith("branch refs/heads/")) {
                if ($line.Substring(18).Trim() -eq $RestoreBranch) {
                    $wtPath = $tempP
                }
            } elseif ([string]::IsNullOrWhiteSpace($line)) {
                $tempP = ""
            }
        }
    }

    if ($wtPath) {
        Write-Host "Branch is checked out in Worktree ($wtPath), performing worktree reset..." -ForegroundColor DarkGray
        $r = Invoke-Git @("-C", $wtPath, "reset", "--hard", $targetSha)
        if ($r.ExitCode -ne 0) { throw "Reset failed: $($r.Error)" }
    } else {
        $r = Invoke-Git @("branch", "-f", $RestoreBranch, $targetSha)
        if ($r.ExitCode -ne 0) { throw "Updating branch pointer failed: $($r.Error)" }
    }
    Write-Host "✅ Branch '$RestoreBranch' successfully restored to backup $BackupRef ($targetSha)" -ForegroundColor Green
    return
}

if ($Clean) {
    Write-Host ">> Cleaning expired safety backups (older than $OlderThanDays days)..." -ForegroundColor Cyan
    $allBackups = Invoke-Git @("for-each-ref", "--format=%(refname)|%(committerdate:unix)", "refs/sync-backup/")
    $nowEpoch = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $cutoff = $nowEpoch - ($OlderThanDays * 86400)
    $deletedCount = 0

    if ($allBackups.ExitCode -eq 0 -and (-not [string]::IsNullOrWhiteSpace($allBackups.Output))) {
        foreach ($line in ($allBackups.Output -split "`r?`n")) {
            if ([string]::IsNullOrWhiteSpace($line)) { continue }
            $parts = $line.Split('|')
            $ref = $parts[0].Trim()
            $epoch = [long]0
            if ($parts.Length -gt 1) { [long]::TryParse($parts[1].Trim(), [ref]$epoch) | Out-Null }
            if ($epoch -le $cutoff -or $OlderThanDays -eq 0) {
                Invoke-Git @("update-ref", "-d", $ref) | Out-Null
                $deletedCount++
            }
        }
    }
    Write-Host "✅ Cleaned $deletedCount expired backups." -ForegroundColor Green
    return
}

# Default mode: list all backups
Write-Host ""
Write-Host "===================== refs/sync-backup/ Inventory =====================" -ForegroundColor Cyan
$backups = Invoke-Git @("for-each-ref", "--sort=-committerdate", "--format=%(refname)|%(objectname:short)|%(committerdate:iso8601)|%(subject)", "refs/sync-backup/")

if ($backups.ExitCode -ne 0 -or [string]::IsNullOrWhiteSpace($backups.Output)) {
    Write-Host "No safety backup records found." -ForegroundColor Yellow
} else {
    $fmt = "{0,-42} {1,-10} {2,-20} {3}"
    Write-Host ([string]::Format($fmt, "Backup Ref", "SHA", "Created At", "Subject")) -ForegroundColor DarkGray
    Write-Host ("-" * 86) -ForegroundColor DarkGray
    foreach ($line in ($backups.Output -split "`r?`n")) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $parts = $line.Split('|')
        $r = $parts[0].Trim()
        $s = if ($parts.Length -gt 1) { $parts[1].Trim() } else { "" }
        $d = if ($parts.Length -gt 2) { $parts[2].Trim() } else { "" }
        $subj = if ($parts.Length -gt 3) { $parts[3].Trim() } else { "" }
        Write-Host ([string]::Format($fmt, $r, $s, $d, $subj)) -ForegroundColor White
    }
}
Write-Host "----------------------------------------------------------------------"
Write-Host "Restore example:" -ForegroundColor DarkGray
Write-Host "  pwsh .agents/skills/git-workflow/scripts/manage-sync-backups.ps1 -RestoreBranch 'dev' -BackupRef 'refs/sync-backup/...' " -ForegroundColor White
Write-Host "======================================================================" -ForegroundColor Cyan
Write-Host ""