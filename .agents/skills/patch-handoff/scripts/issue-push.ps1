#Requires -Version 7.0
<#
.SYNOPSIS
    Push local issue files to GitHub, wiring sub-issue parents and native blocked_by edges.
.DESCRIPTION
    Reads Markdown files whose front matter carries key / title / labels / parent / blocked_by.
    Idempotent: existing edges are read before writing. Chunked: respects a wall-clock budget
    and leaves a resumable state file. Every API call retries on transport errors.
.EXAMPLE
    pwsh issue-push.ps1 -Repo owner/name -InputDir ./issue-plan -DryRun
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Repo,
    # Not named -Input: that collides with the automatic $input variable.
    [Parameter(Mandatory)][string]$InputDir,
    [string]$Mapping = './issue-mapping.json',
    [switch]$DryRun,
    [switch]$Resume,
    [int]$BudgetSeconds = 110
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'issue-common.ps1')

Assert-RepoFormat -Repo $Repo
if (-not $DryRun) { Assert-GhAvailable }

$inputDir = [System.IO.Path]::GetFullPath($InputDir, (Get-Location).ProviderPath)
if (-not (Test-Path -LiteralPath $inputDir)) { throw "Input directory not found: $inputDir" }
$mappingPath = [System.IO.Path]::GetFullPath($Mapping, (Get-Location).ProviderPath)
$statePath = [System.IO.Path]::ChangeExtension($mappingPath, '.state.json')

$deadline = [DateTime]::UtcNow.AddSeconds($BudgetSeconds)
$dbIdCache = @{}
$createdCount = 0
$linkedCount = 0

if ($Resume) {
    if (Test-Path -LiteralPath $statePath) {
        $state = [System.IO.File]::ReadAllText($statePath) | ConvertFrom-Json
        Write-Host "Resuming: $($state.pending_creates.Count) pending creates, $($state.pending_edges.Count) pending edges from prior run."
    }
    else {
        Write-Host 'No resume state found; continuing from the mapping file.'
    }
}

function Test-Budget {
    if ([DateTime]::UtcNow -ge $deadline) { return $false }
    return $true
}

function Get-IssueDbId {
    param([int]$Number)
    if (-not $dbIdCache.ContainsKey($Number)) {
        $json = Invoke-Gh -GhArgs @('api', "repos/$Repo/issues/$Number", '--jq', '.id')
        $dbIdCache[$Number] = [int64]$json.Trim()
    }
    return $dbIdCache[$Number]
}

# Relation reads are cached per endpoint: a re-run must verify every candidate without
# spending its whole budget on repeated GETs of the same parent or child.
$subIssueCache = @{}
$blockedByCache = @{}

function Get-LinkedSubIssues {
    param([int]$ParentNumber)
    if (-not $subIssueCache.ContainsKey($ParentNumber)) {
        $json = Invoke-Gh -GhArgs @('api', "repos/$Repo/issues/$ParentNumber/sub_issues", '--jq', '[.[].number]')
        $numbers = if ([string]::IsNullOrWhiteSpace($json) -or $json.Trim() -eq 'null') { @() } else { @($json | ConvertFrom-Json | ForEach-Object { [int]$_ }) }
        $subIssueCache[$ParentNumber] = $numbers
    }
    return , $subIssueCache[$ParentNumber]
}

function Add-LinkedSubIssueCache {
    param([int]$ParentNumber, [int]$ChildNumber)
    $existing = @($subIssueCache[$ParentNumber])
    $subIssueCache[$ParentNumber] = @($existing + $ChildNumber)
}

function Get-BlockedByNumbers {
    param([int]$ChildNumber)
    if (-not $blockedByCache.ContainsKey($ChildNumber)) {
        $json = Invoke-Gh -GhArgs @('api', "repos/$Repo/issues/$ChildNumber/dependencies/blocked_by", '--jq', '[.[].number]')
        $numbers = if ([string]::IsNullOrWhiteSpace($json) -or $json.Trim() -eq 'null') { @() } else { @($json | ConvertFrom-Json | ForEach-Object { [int]$_ }) }
        $blockedByCache[$ChildNumber] = $numbers
    }
    return , $blockedByCache[$ChildNumber]
}

function Add-BlockedByCache {
    param([int]$ChildNumber, [int]$BlockerNumber)
    $existing = @($blockedByCache[$ChildNumber])
    $blockedByCache[$ChildNumber] = @($existing + $BlockerNumber)
}

# ---------- load input files ----------
$files = Get-ChildItem -LiteralPath $inputDir -Filter '*.md' -File -Recurse | Sort-Object FullName
if ($files.Count -eq 0) { throw "No Markdown files found under $inputDir" }

$nodes = [ordered]@{}
foreach ($file in $files) {
    $record = Read-IssueFile -Path $file.FullName
    if (-not $record.Key) { throw "Missing 'key' in front matter: $($file.FullName)" }
    if (-not $record.Title) { throw "Missing 'title' in front matter: $($file.FullName)" }
    if ($nodes.Contains($record.Key)) { throw "Duplicate key '$($record.Key)' at $($file.FullName)" }
    $nodes[$record.Key] = $record
}

foreach ($key in $nodes.Keys) {
    $node = $nodes[$key]
    if ($node.Parent -and -not $nodes.Contains($node.Parent)) {
        throw "Key '$key' references unknown parent '$($node.Parent)'"
    }
    foreach ($b in $node.BlockedBy) {
        if (-not $nodes.Contains($b)) { throw "Key '$key' references unknown blocker '$b'" }
    }
}

# ---------- mapping ----------
$mappingData = [ordered]@{}
if (Test-Path -LiteralPath $mappingPath) {
    $loaded = [System.IO.File]::ReadAllText($mappingPath) | ConvertFrom-Json -AsHashtable
    foreach ($k in $loaded.Keys) { $mappingData[$k] = $loaded[$k] }
}
function Resolve-Number {
    param([string]$Key)
    if ($nodes.Contains($Key) -and $nodes[$Key].Number) { return [int]$nodes[$Key].Number }
    if ($mappingData.Contains($Key) -and $mappingData[$Key].number) { return [int]$mappingData[$Key].number }
    return $null
}

$pendingCreate = @($nodes.Keys | Where-Object { $null -eq (Resolve-Number $_) })
# Parents and edges are planned by key, not by resolved number: on a fresh plan nothing
# has a number yet, and the dry-run must still show the wiring it would perform.
$pendingParents = @($nodes.Keys | Where-Object { $null -ne $nodes[$_].Parent })
$pendingEdges = @()
foreach ($key in $nodes.Keys) {
    foreach ($b in $nodes[$key].BlockedBy) {
        $pendingEdges += , @($key, $b)
    }
}

Write-Host "Plan: $($pendingCreate.Count) to create, $($pendingParents.Count) parent links, $($pendingEdges.Count) dependency edges."
if ($DryRun) {
    foreach ($key in $pendingCreate) { Write-Host "[dry-run] create $key :: $($nodes[$key].Title)" }
    foreach ($key in $pendingParents) { Write-Host "[dry-run] sub-issue $($nodes[$key].Parent) -> $key" }
    foreach ($edge in $pendingEdges) { Write-Host "[dry-run] blocked-by $($edge[0]) <- $($edge[1])" }
    Write-Host '[dry-run] no remote calls with side effects were made.'
    return
}

function Save-Mapping {
    foreach ($key in $nodes.Keys) {
        $number = Resolve-Number $key
        if ($null -eq $number) { continue }
        $mappingData[$key] = [ordered]@{
            number = $number
            url    = "https://github.com/$Repo/issues/$number"
        }
    }
    $json = [ordered]@{}
    foreach ($k in ($mappingData.Keys | Sort-Object)) { $json[$k] = $mappingData[$k] }
    Write-Utf8File -Path $mappingPath -Content ($json | ConvertTo-Json -Depth 5)
}

function Save-State {
    param([string[]]$PendingCreates, [string[]]$PendingParentKeys, $PendingEdgePairs)
    $state = [ordered]@{
        pending_creates = @($PendingCreates)
        pending_parents = @($PendingParentKeys)
        pending_edges   = @($PendingEdgePairs | ForEach-Object { , @($_[0], $_[1]) })
    }
    Write-Utf8File -Path $statePath -Content ($state | ConvertTo-Json -Depth 6)
}

# ---------- phase 1: create ----------
$createdNow = @{}
$remainingCreates = @()
foreach ($key in $pendingCreate) {
    if (-not (Test-Budget)) { $remainingCreates += $key; continue }
    $node = $nodes[$key]
    $bodyFile = [System.IO.Path]::GetTempFileName()
    try {
        Write-Utf8File -Path $bodyFile -Content $node.Body
        $ghArgs = @('issue', 'create', '--repo', $Repo, '--title', $node.Title, '--body-file', $bodyFile)
        foreach ($label in $node.Labels) { $ghArgs += @('--label', $label) }
        $url = Invoke-Gh -GhArgs $ghArgs
        if ($url -notmatch '/issues/(\d+)\s*$') { throw "Cannot parse issue number from: $url" }
        $number = [int]$Matches[1]
        $nodes[$key].Number = $number
        $mappingData[$key] = [ordered]@{ number = $number; url = $url.Trim() }
        $createdNow[$key] = $number
        $createdCount++
        Write-Host "created $key -> #$number"
        Save-Mapping
    }
    finally {
        Remove-Item -LiteralPath $bodyFile -Force -ErrorAction SilentlyContinue
    }
}

if ($remainingCreates.Count -gt 0) {
    Save-Mapping
    $stillPending = @($remainingCreates | ForEach-Object { $_.ToString() })
    Save-State -PendingCreates $stillPending -PendingParentKeys @() -PendingEdgePairs @()
    Write-Warning "Budget exhausted after creating $createdCount issues. Re-run with -Resume to continue."
    exit 2
}

# ---------- phase 2: parent links ----------
$remainingParents = @()
foreach ($key in $pendingParents) {
    if (-not (Test-Budget)) { $remainingParents += $key; continue }
    $childNumber = Resolve-Number $key
    $parentNumber = Resolve-Number $nodes[$key].Parent
    if ($null -eq $childNumber -or $null -eq $parentNumber) { continue }
    $existing = Get-LinkedSubIssues -ParentNumber $parentNumber
    if ($existing -contains $childNumber) { continue }
    $childDbId = Get-IssueDbId $childNumber
    Invoke-Gh -GhArgs @('api', '--method', 'POST', "repos/$Repo/issues/$parentNumber/sub_issues", '-F', "sub_issue_id=$childDbId") | Out-Null
    Add-LinkedSubIssueCache -ParentNumber $parentNumber -ChildNumber $childNumber
    $linkedCount++
    Write-Host "sub-issue #$parentNumber -> #$childNumber ($key)"
}

# ---------- phase 3: dependency edges ----------
$remainingEdges = @()
foreach ($edge in $pendingEdges) {
    if (-not (Test-Budget)) { $remainingEdges += , $edge; continue }
    $childNumber = Resolve-Number $edge[0]
    $blockerNumber = Resolve-Number $edge[1]
    if ($null -eq $childNumber -or $null -eq $blockerNumber) { continue }
    $existing = Get-BlockedByNumbers -ChildNumber $childNumber
    if ($existing -contains $blockerNumber) { continue }
    $blockerDbId = Get-IssueDbId $blockerNumber
    Invoke-Gh -GhArgs @('api', '--method', 'POST', "repos/$Repo/issues/$childNumber/dependencies/blocked_by", '-F', "issue_id=$blockerDbId") | Out-Null
    Add-BlockedByCache -ChildNumber $childNumber -BlockerNumber $blockerNumber
    $linkedCount++
    Write-Host "blocked-by #$childNumber <- #$blockerNumber ($($edge[0]) <- $($edge[1]))"
}

Save-Mapping

if ($remainingParents.Count -gt 0 -or $remainingEdges.Count -gt 0) {
    Save-State -PendingCreates @() -PendingParentKeys $remainingParents -PendingEdgePairs $remainingEdges
    Write-Warning "Budget exhausted. Created $createdCount, linked $linkedCount. Re-run with -Resume to finish."
    exit 2
}

if (Test-Path -LiteralPath $statePath) { Remove-Item -LiteralPath $statePath -Force }
Write-Host "Done. Created $createdCount issues, wired $linkedCount links. Mapping: $mappingPath"