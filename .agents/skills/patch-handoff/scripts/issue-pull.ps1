#Requires -Version 7.0
<#
.SYNOPSIS
    Pull GitHub issues into a standalone Markdown bundle with index, topology, and raw JSON.
.EXAMPLE
    pwsh issue-pull.ps1 -Repo owner/name -Out ./issue-export -Zip
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Repo,
    [ValidateSet('open', 'closed', 'all')][string]$State = 'open',
    [string[]]$Label,
    [string]$Search,
    [int]$Limit = 500,
    [string]$Out = './issue-export',
    [switch]$Zip,
    [string]$ZipPath,
    [string]$Templates,
    [string[]]$KindRules
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'issue-common.ps1')

Assert-GhAvailable
Assert-RepoFormat -Repo $Repo

$outDir = [System.IO.Path]::GetFullPath($Out, (Get-Location).ProviderPath)
if ($outDir.TrimEnd('\', '/') -eq (Get-Location).ProviderPath.TrimEnd('\', '/')) {
    throw 'Refusing to export into the current directory; pass -Out with a dedicated folder.'
}
if (Test-Path -LiteralPath (Join-Path $outDir '.git')) {
    throw "Refusing to overwrite a repository work tree: $outDir"
}

$ghArgs = @('issue', 'list', '--repo', $Repo, '--state', $State, '--limit', "$Limit", '--json', ($IssueJsonFields -join ','))
foreach ($l in @($Label | Where-Object { $_ })) { $ghArgs += @('--label', $l) }
if ($Search) { $ghArgs += @('--search', $Search) }

Write-Host "Fetching issues from $Repo (state=$State)..."
$rawText = Invoke-Gh -GhArgs $ghArgs
$raw = if ([string]::IsNullOrWhiteSpace($rawText)) { @() } else { $rawText | ConvertFrom-Json }
$raw = @($raw)
if ($raw.Count -eq 0) {
    Write-Warning 'No issues matched the filter; nothing to export.'
    return
}

$records = @()
foreach ($issue in $raw) {
    $parent = if ($null -ne $issue.parent) { [int]$issue.parent.number } else { $null }
    $children = Get-ConnectionNumbers $issue.subIssues
    $records += [pscustomobject]@{
        Number    = [int]$issue.number
        Title     = [string]$issue.title
        Body      = [string]$issue.body
        State     = [string]$issue.state
        Url       = [string]$issue.url
        Author    = if ($null -ne $issue.author) { [string]$issue.author.login } else { '' }
        CreatedAt = Format-Timestamp $issue.createdAt
        UpdatedAt = Format-Timestamp $issue.updatedAt
        Labels    = @($issue.labels | ForEach-Object { [string]$_.name })
        Parent    = $parent
        Children  = $children
        BlockedBy = (Get-ConnectionNumbers $issue.blockedBy)
        Blocking  = (Get-ConnectionNumbers $issue.blocking)
        Comments  = @($issue.comments)
        Kind      = Get-IssueKind -Title ([string]$issue.title) -Children $children -Parent $parent -KindRules $KindRules
    }
}

$byNumber = @{}
foreach ($r in $records) { $byNumber[$r.Number] = $r }

$blocks = @{}
foreach ($r in $records) { $blocks[$r.Number] = @() }
foreach ($r in $records) {
    foreach ($b in $r.BlockedBy) {
        if ($blocks.ContainsKey($b)) { $blocks[$b] += $r.Number }
    }
}

function Format-Ref {
    param([int]$Number)
    if ($byNumber.ContainsKey($Number)) {
        return "[#$Number]($($byNumber[$Number].Url)) $($byNumber[$Number].Title)"
    }
    return "#$Number"
}

function Format-ShortRef {
    param([int]$Number)
    if (-not $byNumber.ContainsKey($Number)) { return "#$Number" }
    return "[#$Number]($($byNumber[$Number].Url))"
}

function Format-MetaTable {
    param($Record)
    $labelText = if (@($Record.Labels).Count -gt 0) { ($Record.Labels | ForEach-Object { "``$_``" }) -join ', ' } else { '-' }
    $parentText = if ($null -ne $Record.Parent) { Format-ShortRef $Record.Parent } else { '-' }
    $visibleChildren = @($Record.Children | Where-Object { $byNumber.ContainsKey($_) })
    $visibleBlockers = @($Record.BlockedBy | Where-Object { $byNumber.ContainsKey($_) })
    $childText = if ($visibleChildren.Count -gt 0) { ($visibleChildren | ForEach-Object { Format-ShortRef $_ }) -join ', ' } else { '-' }
    $blockedByText = if ($visibleBlockers.Count -gt 0) { ($visibleBlockers | ForEach-Object { Format-ShortRef $_ }) -join ', ' } else { '-' }
    $blocksText = if (@($blocks[$Record.Number]).Count -gt 0) { ($blocks[$Record.Number] | ForEach-Object { Format-ShortRef $_ }) -join ', ' } else { '-' }

    $lines = @(
        '| Field | Value |'
        '| :--- | :--- |'
        "| Number | #$($Record.Number) |"
        "| Kind | $($Record.Kind) |"
        "| State | $($Record.State) |"
        "| Labels | $labelText |"
        "| URL | $($Record.Url) |"
        "| Author | $($Record.Author) |"
        "| Parent | $parentText |"
        "| Children | $childText |"
        "| Blocked by | $blockedByText |"
        "| Blocks | $blocksText |"
        "| Created | $($Record.CreatedAt) |"
        "| Updated | $($Record.UpdatedAt) |"
    )
    return ($lines -join "`n")
}

function Format-Comments {
    param($Record)
    if (@($Record.Comments).Count -eq 0) { return '' }
    $parts = @('## Comments')
    foreach ($c in $Record.Comments) {
        $author = if ($null -ne $c.author) { [string]$c.author.login } else { 'unknown' }
        $parts += ''
        $parts += "### $author · $($c.createdAt)"
        $parts += ''
        $parts += ([string]$c.body).TrimEnd()
    }
    return ($parts -join "`n")
}

function Format-FrontMatter {
    param($Record)
    $lines = @('---')
    $lines += "key: $($Record.Number)"
    $lines += "number: $($Record.Number)"
    $lines += "title: $(Format-YamlString $Record.Title)"
    $lines += "kind: $($Record.Kind)"
    $lines += "state: $($Record.State)"
    $lines += "labels: $(Format-YamlList $Record.Labels)"
    if ($null -ne $Record.Parent) { $lines += "parent: $($Record.Parent)" }
    if (@($Record.BlockedBy).Count -gt 0) { $lines += "blocked_by: [$(($Record.BlockedBy | ForEach-Object { $_ }) -join ', ')]" }
    if (@($blocks[$Record.Number]).Count -gt 0) { $lines += "blocks: [$(($blocks[$Record.Number]) -join ', ')]" }
    $lines += "url: $($Record.Url)"
    $lines += "created_at: $($Record.CreatedAt)"
    $lines += "updated_at: $($Record.UpdatedAt)"
    $lines += '---'
    return ($lines -join "`n")
}

$defaultTemplates = Join-Path (Split-Path -Parent $PSScriptRoot) 'templates'
$issueTemplate = Get-TemplatePath -TemplateDir $Templates -Name 'issue.md' -DefaultDir $defaultTemplates
$indexTemplate = Get-TemplatePath -TemplateDir $Templates -Name 'index.md' -DefaultDir $defaultTemplates
$topologyTemplate = Get-TemplatePath -TemplateDir $Templates -Name 'topology.md' -DefaultDir $defaultTemplates

if (Test-Path -LiteralPath $outDir) { Remove-Item -LiteralPath $outDir -Recurse -Force }
foreach ($sub in @('issues', 'data')) {
    New-Item -ItemType Directory -Path (Join-Path $outDir $sub) -Force | Out-Null
}

$issueTemplateText = [System.IO.File]::ReadAllText($issueTemplate)
$indexRows = @()
$indexRows += '| Number | Kind | Title | Blocked by | Children | File |'
$indexRows += '| :--- | :--- | :--- | :--- | :--- | :--- |'

foreach ($r in ($records | Sort-Object Number)) {
    $fname = '{0:D4}-{1}-{2}.md' -f $r.Number, $r.Kind, (Get-Slug $r.Title)
    $doc = Expand-MiniTemplate -Text $issueTemplateText -Values @{
        FRONT_MATTER = (Format-FrontMatter $r)
        TITLE        = $r.Title
        META_TABLE   = (Format-MetaTable $r)
        BODY         = $r.Body.TrimEnd()
        COMMENTS     = (Format-Comments $r)
    }
    $doc = $doc.TrimStart([char[]]"`r`n`t ")
    Write-Utf8File -Path (Join-Path (Join-Path $outDir 'issues') $fname) -Content $doc

    $visibleBlockers = @($r.BlockedBy | Where-Object { $byNumber.ContainsKey($_) })
    $visibleChildren = @($r.Children | Where-Object { $byNumber.ContainsKey($_) })
    $blockedByText = if ($visibleBlockers.Count -gt 0) { ($visibleBlockers | ForEach-Object { "#$_" }) -join ', ' } else { '-' }
    $childText = if ($visibleChildren.Count -gt 0) { ($visibleChildren | ForEach-Object { "#$_" }) -join ', ' } else { '-' }
    $indexRows += "| [#$($r.Number)]($($r.Url)) | $($r.Kind) | $($r.Title) | $blockedByText | $childText | [$('{0:D4}' -f $r.Number)](<issues/$fname>) |"
}

function Format-Tree {
    param([int]$Root, [int]$Depth, [System.Collections.Generic.HashSet[int]]$Seen)
    $pad = '  ' * $Depth
    $line = "$pad- $(Format-Ref $Root)"
    $rendered = @($line)
    [void]$Seen.Add($Root)
    foreach ($child in $byNumber[$Root].Children) {
        if (-not $byNumber.ContainsKey($child)) { continue }
        if ($Seen.Contains($child)) { continue }
        $rendered += Format-Tree -Root $child -Depth ($Depth + 1) -Seen $Seen
    }
    return $rendered
}

$seen = [System.Collections.Generic.HashSet[int]]::new()
$treeLines = @()
$roots = $records | Where-Object { $null -eq $_.Parent -or -not $byNumber.ContainsKey($_.Parent) } | Sort-Object Number
foreach ($root in $roots) {
    if ($seen.Contains($root.Number)) { continue }
    $treeLines += Format-Tree -Root $root.Number -Depth 0 -Seen $seen
}
foreach ($r in ($records | Sort-Object Number)) {
    if ($seen.Contains($r.Number)) { continue }
    $treeLines += Format-Tree -Root $r.Number -Depth 0 -Seen $seen
}

$edgeCount = (@($records | ForEach-Object { @($_.BlockedBy).Count }) | Measure-Object -Sum).Sum
if ($null -eq $edgeCount) { $edgeCount = 0 }

$topologyText = Expand-MiniTemplate `
    -Text ([System.IO.File]::ReadAllText($topologyTemplate)) `
    -Values @{
        TREE = ($treeLines -join "`n")
        NOTE = "$($records.Count) issues, $edgeCount blocked-by edges, $($roots.Count) top-level nodes."
    }

$numbers = $records | ForEach-Object { $_.Number } | Sort-Object
$notes = @(
    "- Source: https://github.com/$Repo/issues"
    "- Pulled with ``gh issue list --json`` and rendered from templates in ``templates/``."
    "- ``data/issues.json`` keeps the raw API payload; ``data/relations.json`` keys every relation by issue number."
) -join "`n"

$readme = Expand-MiniTemplate `
    -Text ([System.IO.File]::ReadAllText($indexTemplate)) `
    -Values @{
        REPO         = $Repo
        COUNT        = [string]$records.Count
        RANGE        = "#$($numbers[0]) ~ #$($numbers[-1])"
        GENERATED_AT = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd HH:mm:ss') + ' UTC'
        TABLE        = ($indexRows -join "`n")
        TOPOLOGY     = $topologyText
        NOTES        = $notes
    }
Write-Utf8File -Path (Join-Path $outDir 'README.md') -Content $readme

$relations = [ordered]@{}
foreach ($r in ($records | Sort-Object Number)) {
    $relations["$($r.Number)"] = [ordered]@{
        parent     = $r.Parent
        sub_issues = $r.Children
        blocked_by = $r.BlockedBy
        blocking   = $r.Blocking
        comments   = @($r.Comments | ForEach-Object {
                [ordered]@{ author = $(if ($null -ne $_.author) { $_.author.login } else { $null }); created_at = $_.createdAt; body = $_.body }
            })
    }
}
Write-Utf8File -Path (Join-Path $outDir 'data/issues.json') -Content $rawText
Write-Utf8File -Path (Join-Path $outDir 'data/relations.json') -Content ($relations | ConvertTo-Json -Depth 8)

$summary = "Exported $($records.Count) issues ($edgeCount blocked-by edges) to $outDir"
if ($Zip) {
    if (-not $ZipPath) {
        $parent = Split-Path -Parent $outDir
        $leaf = Split-Path -Leaf $outDir
        $ZipPath = Join-Path $parent "$leaf.zip"
    }
    $ZipPath = [System.IO.Path]::GetFullPath($ZipPath, (Get-Location).ProviderPath)
    if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($outDir, $ZipPath, [System.IO.Compression.CompressionLevel]::Optimal, $true)
    $summary += "`nArchive: $ZipPath ($([math]::Round((Get-Item $ZipPath).Length / 1KB, 1)) KB)"
}
Write-Host $summary