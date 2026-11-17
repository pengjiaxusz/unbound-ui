#requires -Version 7
<#
.SYNOPSIS
    Clone the read-only WinUI 3 / Fluent reference repositories.

.DESCRIPTION
    Every checkout is shallow (`--depth 1`). Very large repositories use a blobless
    partial clone plus sparse checkout so only the directories that matter for
    control styling land on disk.

    Nothing here is compiled or committed: `reference/.gitignore` ignores every
    entry except this script, the manifest and the notes.

.PARAMETER Only
    One or more directory names from the manifest, e.g. `-Only microsoft-ui-xaml,WinUI-Gallery`.

.PARAMETER Force
    Delete an existing checkout and clone it again.

.EXAMPLE
    pwsh -NoProfile -File reference/clone.ps1

.EXAMPLE
    pwsh -NoProfile -File reference/clone.ps1 -Only FluentAvalonia,wpfui -Force
#>
[CmdletBinding()]
param(
    [string[]]$Only = @(),
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

# Directory = repository name shorthand used by -Only. Sparse lists only the
# directories that carry styling/interaction facts; empty means full shallow clone.
$repos = @(
    # WinUI 3 的控件模板与主题字典位于 controls/dev（每个控件一个 *_themeresources.xaml），
    # 默认模板与颜色资源另在 dxaml/xcp/dxaml/themes。
    @{ Name = 'microsoft-ui-xaml'; Url = 'https://github.com/microsoft/microsoft-ui-xaml.git'; Sparse = @('controls/dev', 'dxaml/xcp/dxaml/themes') }
    @{ Name = 'WinUI-Gallery'; Url = 'https://github.com/microsoft/WinUI-Gallery.git'; Sparse = @() }
    @{ Name = 'WindowsAppSDK-Samples'; Url = 'https://github.com/microsoft/WindowsAppSDK-Samples.git'; Sparse = @('Samples') }
    @{ Name = 'fluentui'; Url = 'https://github.com/microsoft/fluentui.git'; Sparse = @(
            'packages/tokens',
            'packages/theme',
            'packages/react-components/react-button',
            'packages/react-components/react-checkbox',
            'packages/react-components/react-switch',
            'packages/react-components/react-slider',
            'packages/react-components/react-tabs',
            'packages/react-components/react-menu',
            'packages/react-components/react-tooltip',
            'packages/react-components/react-infobar',
            'packages/react-components/react-dialog',
            'packages/react-components/react-input',
            'packages/react-components/react-select',
            'packages/react-components/react-progress',
            'packages/react-components/react-spinner'
        ) }
    @{ Name = 'fluentui-system-icons'; Url = 'https://github.com/microsoft/fluentui-system-icons.git'; Sparse = @('fonts') }
    @{ Name = 'FluentAvalonia'; Url = 'https://github.com/amwx/FluentAvalonia.git'; Sparse = @() }
    @{ Name = 'wpfui'; Url = 'https://github.com/lepoco/wpfui.git'; Sparse = @() }
    @{ Name = 'ModernWpf'; Url = 'https://github.com/Kinnara/ModernWpf.git'; Sparse = @() }
    @{ Name = 'Windows'; Url = 'https://github.com/CommunityToolkit/Windows.git'; Sparse = @() }
    @{ Name = 'Labs-Windows'; Url = 'https://github.com/CommunityToolkit/Labs-Windows.git'; Sparse = @() }
    @{ Name = 'DevWinUI'; Url = 'https://github.com/ghost1372/DevWinUI.git'; Sparse = @() }
    @{ Name = 'WinUI.TableView'; Url = 'https://github.com/w-ahmad/WinUI.TableView.git'; Sparse = @() }
    @{ Name = 'WinUIEx'; Url = 'https://github.com/dotMorten/WinUIEx.git'; Sparse = @() }
    @{ Name = 'MicaForEveryone'; Url = 'https://github.com/MicaForEveryone/MicaForEveryone.git'; Sparse = @() }
    @{ Name = 'Files'; Url = 'https://github.com/files-community/Files.git'; Sparse = @('src/Files.App') }
    @{ Name = 'PowerToys'; Url = 'https://github.com/microsoft/PowerToys.git'; Sparse = @('src/settings-ui', 'src/common') }
    @{ Name = 'DevToys'; Url = 'https://github.com/DevToys-app/DevToys.git'; Sparse = @('src') }
)

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw 'git is required but was not found on PATH.'
}

# Git LFS assets are irrelevant for reading styles and would only slow the clone down.
$env:GIT_LFS_SKIP_SMUDGE = '1'

$selected = if ($Only.Count -gt 0) {
    $known = $repos | ForEach-Object { $_.Name }
    foreach ($name in $Only) {
        if ($known -notcontains $name) {
            throw "Unknown reference repository '$name'. Known: $($known -join ', ')"
        }
    }
    $repos | Where-Object { $Only -contains $_.Name }
}
else {
    $repos
}

$cloned = @()
$skipped = @()
$failed = @()

foreach ($repo in $selected) {
    $target = Join-Path $root $repo.Name
    if (Test-Path -LiteralPath $target) {
        if (-not $Force) {
            Write-Host "[skip] $($repo.Name) already exists" -ForegroundColor DarkGray
            $skipped += $repo.Name
            continue
        }
        Write-Host "[reset] $($repo.Name)" -ForegroundColor Yellow
        Remove-Item -LiteralPath $target -Recurse -Force
    }

    $sparse = $repo.Sparse.Count -gt 0
    $args = @('clone', '--depth', '1', '--single-branch', '--no-tags')
    if ($sparse) {
        $args += @('--filter=blob:none', '--sparse')
    }
    $args += @($repo.Url, $target)

    Write-Host "[clone] $($repo.Name)" -ForegroundColor Cyan
    try {
        & git @args
        if ($LASTEXITCODE -ne 0) {
            throw "git clone exited with $LASTEXITCODE"
        }
        if ($sparse) {
            & git -C $target sparse-checkout set @($repo.Sparse)
            if ($LASTEXITCODE -ne 0) {
                throw "git sparse-checkout exited with $LASTEXITCODE"
            }
        }
        $cloned += $repo.Name
    }
    catch {
        Write-Host "[fail ] $($repo.Name): $($_.Exception.Message)" -ForegroundColor Red
        $failed += $repo.Name
    }
}

Write-Host ''
Write-Host ("cloned: {0} | skipped: {1} | failed: {2}" -f $cloned.Count, $skipped.Count, $failed.Count)
if ($failed.Count -gt 0) {
    Write-Host ("failed: {0}" -f ($failed -join ', ')) -ForegroundColor Red
    exit 1
}
