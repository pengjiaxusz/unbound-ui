# Shared helpers for the issue scripts. Dot-source this file; do not run it.
# All identifiers, comments, and log lines are English by repo convention.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Native process output must round-trip UTF-8. Without this, non-ASCII titles are
# decoded with the console code page and the JSON payload becomes unparseable.
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = $utf8
[Console]::InputEncoding = $utf8
$OutputEncoding = $utf8

$script:IssueJsonFields = @(
    'number', 'title', 'body', 'state', 'stateReason', 'labels', 'url', 'author',
    'createdAt', 'updatedAt', 'parent', 'subIssues', 'blockedBy', 'blocking', 'comments'
)

function Assert-GhAvailable {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw 'gh CLI not found. Install GitHub CLI and run: gh auth login'
    }
    $auth = & gh auth status 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "gh is not authenticated. Run: gh auth login`n$($auth | Out-String)"
    }
}

function Assert-RepoFormat {
    param([Parameter(Mandatory)][string]$Repo)
    if ($Repo -notmatch '^[^/\s]+/[^/\s]+$') {
        throw "Repo must look like owner/name, got: $Repo"
    }
}

function Invoke-Gh {
    param(
        [Parameter(Mandatory)][string[]]$GhArgs,
        [int]$Tries = 4,
        [int]$BackoffSeconds = 2
    )
    for ($attempt = 1; $attempt -le $Tries; $attempt++) {
        $output = & gh @GhArgs 2>&1
        if ($LASTEXITCODE -eq 0) { return ($output | Out-String).Trim() }
        $message = ($output | Out-String).Trim()
        if ($attempt -eq $Tries) {
            throw "gh failed after $Tries attempts: gh $($GhArgs -join ' ')`n$message"
        }
        Write-Warning "gh attempt $attempt/$Tries failed, retrying: $message"
        Start-Sleep -Seconds $BackoffSeconds
    }
}

function Get-ConnectionNumbers {
    param($Connection)
    # Return the array as a single object so an empty result stays an empty array
    # instead of collapsing to $null (which would make @($null).Count equal 1).
    $numbers = @()
    if ($null -ne $Connection -and $null -ne $Connection.nodes) {
        $numbers = @($Connection.nodes | ForEach-Object { [int]$_.number })
    }
    return , $numbers
}

function Get-IssueKind {
    param(
        [string]$Title,
        [int[]]$Children,
        $Parent,
        [string[]]$KindRules
    )
    if ($KindRules) {
        foreach ($rule in $KindRules) {
            $parts = $rule.Split('=', 2)
            if ($parts.Count -ne 2) { continue }
            if ($Title -match $parts[1]) { return $parts[0].Trim() }
        }
    }
    $childArray = @($Children | Where-Object { $null -ne $_ })
    if ($childArray.Count -gt 0) { return 'epic' }
    if ($null -ne $Parent) { return 'ticket' }
    return 'issue'
}

function Get-Slug {
    param([string]$Text, [int]$MaxLength = 48)
    $slug = $Text -replace '[\\/:*?"<>|\r\n\t]+', '-'
    $slug = $slug -replace '[：，、（）【】]', '-'
    $slug = $slug -replace '\s+', '-'
    $slug = $slug -replace '-{2,}', '-'
    $slug = $slug.Trim('-', '.', ' ')
    if ($slug.Length -gt $MaxLength) { $slug = $slug.Substring(0, $MaxLength).TrimEnd('-') }
    if ([string]::IsNullOrWhiteSpace($slug)) { $slug = 'issue' }
    return $slug
}

function Format-Timestamp {
    param($Value)
    if ($null -eq $Value) { return '' }
    if ($Value -is [datetime]) { return $Value.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }
    return [string]$Value
}

function Format-YamlString {
    param([string]$Value)
    if ($null -eq $Value) { return '""' }
    return '"' + ($Value -replace '\\', '\\\\' -replace '"', '\"') + '"'
}

function Format-YamlList {
    param([string[]]$Values)
    $items = @($Values | Where-Object { $null -ne $_ -and "$_" -ne '' })
    if ($items.Count -eq 0) { return '[]' }
    return '[' + (($items | ForEach-Object { Format-YamlString ([string]$_) }) -join ', ') + ']'
}

function Expand-MiniTemplate {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][hashtable]$Values
    )
    # Braces are built from code points so the template engine never mistakes them
    # for one of its own delimiters when this script itself is rendered.
    $braceOpen = [string]([char]0x7B) + [string]([char]0x7B)
    $braceClose = [string]([char]0x7D) + [string]([char]0x7D)
    $result = $Text
    foreach ($key in $Values.Keys) {
        $token = $braceOpen + $key + $braceClose
        $value = [string]$Values[$key]
        $result = $result.Replace($token, $value)
    }
    return $result
}

function Get-TemplatePath {
    param(
        [string]$TemplateDir,
        [string]$Name,
        [Parameter(Mandatory)][string]$DefaultDir
    )
    $dir = if ($TemplateDir) { $TemplateDir } else { $DefaultDir }
    $path = Join-Path $dir $Name
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Template not found: $path"
    }
    return $path
}

function Write-Utf8File {
    param([string]$Path, [string]$Content)
    $parent = Split-Path -Parent $Path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [System.IO.File]::WriteAllText($Path, $Content, (New-Object System.Text.UTF8Encoding($false)))
}

function Read-IssueFile {
    param([Parameter(Mandatory)][string]$Path)
    $text = [System.IO.File]::ReadAllText($Path)
    $record = [ordered]@{
        Path = $Path; Key = $null; Title = $null; Labels = @(); Parent = $null
        BlockedBy = @(); Body = $text; Number = $null; State = $null
    }
    if (-not $text.StartsWith('---')) { return [pscustomobject]$record }

    $lines = $text -split "`r?`n"
    $endIndex = -1
    for ($i = 1; $i -lt $lines.Count; $i++) {
        if ($lines[$i].Trim() -eq '---') { $endIndex = $i; break }
    }
    if ($endIndex -lt 0) { return [pscustomobject]$record }

    for ($i = 1; $i -lt $endIndex; $i++) {
        $line = $lines[$i]
        if ($line -match '^\s*#' -or [string]::IsNullOrWhiteSpace($line)) { continue }
        $sep = $line.IndexOf(':')
        if ($sep -lt 1) { continue }
        $name = $line.Substring(0, $sep).Trim()
        $raw = $line.Substring($sep + 1).Trim()

        if ($raw.StartsWith('[') -and $raw.EndsWith(']')) {
            $inner = $raw.Substring(1, $raw.Length - 2).Trim()
            $values = @()
            if ($inner -ne '') {
                $values = [regex]::Matches($inner, '"((?:[^"\\]|\\.)*)"|([^,\s]+)') | ForEach-Object {
                    if ($_.Groups[1].Success) { $_.Groups[1].Value -replace '\\"', '"' } else { $_.Groups[2].Value }
                }
            }
            switch ($name) {
                'labels' { $record.Labels = @($values) }
                'blocked_by' { $record.BlockedBy = @($values) }
                default { }
            }
            continue
        }

        $value = $raw.Trim('"')
        switch ($name) {
            'key' { $record.Key = $value }
            'title' { $record.Title = $value }
            'parent' { if ($value -ne '' -and $value -ne 'null') { $record.Parent = $value } }
            'number' { if ($value -match '^\d+$') { $record.Number = [int]$value } }
            'state' { $record.State = $value }
            default { }
        }
    }

    $bodyLines = if ($endIndex + 1 -lt $lines.Count) { $lines[($endIndex + 1)..($lines.Count - 1)] } else { @() }
    $body = ($bodyLines -join "`n").TrimStart("`r", "`n")
    $record.Body = $body
    if (-not $record.Key -and $record.Number) { $record.Key = [string]$record.Number }
    return [pscustomobject]$record
}