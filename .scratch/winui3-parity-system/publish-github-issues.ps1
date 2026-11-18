#!/usr/bin/env pwsh
#requires -Version 7
<#
.SYNOPSIS
    Publish the WinUI 3 Parity System Spec and 10 Tracer-Bullet Tickets to GitHub Issues
    with native sub-issue links and blocking dependencies (`docs/agents/issue-tracker.md`).
#>
$ErrorActionPreference = 'Stop'

$repoInfo = gh repo view --json nameWithOwner | ConvertFrom-Json
Write-Host "Publishing Spec & 10 Tickets to $($repoInfo.nameWithOwner)..." -ForegroundColor Cyan

# 1. Create Parent Spec Issue
$specUrl = gh issue create `
    --title "spec(widgets): WinUI 3 控件体系对齐、拼版像素差分与 Fluent 设计规律沉淀" `
    --label "ready-for-agent" `
    --body-file ".scratch/winui3-parity-system/SPEC.md"
$specNum = ($specUrl -split '/')[-1].Trim()
Write-Host "Created Parent Spec Issue #$specNum ($specUrl)" -ForegroundColor Green

$tickets = @(
    @{ Id = "01"; File = ".scratch/winui3-parity-system/issues/01-batched-contact-sheet-roi-diff-and-cache.md"; Title = "feat(parity): 01 落地状态拼版截图、有效对比区（ROI）四重门禁与双端用例指纹缓存"; BlockedBy = @() },
    @{ Id = "02"; File = ".scratch/winui3-parity-system/issues/02-control-inventory-and-canonical-name-expand.md"; Title = "feat(widgets): 02 确立三合一控件清单并以扩展模式接入 WinUI 3 规范名映射"; BlockedBy = @() },
    @{ Id = "03"; File = ".scratch/winui3-parity-system/issues/03-pilot-six-controls-roi-zero-defect.md"; Title = "fix(widgets): 03 校准首批 6 个试点基准控件通过有效对比区（ROI）四重门禁"; BlockedBy = @("01") },
    @{ Id = "04"; File = ".scratch/winui3-parity-system/issues/04-parity-batch-1-basic-and-text-inputs.md"; Title = "feat(widgets): 04 接入并校准第一批已实现基准控件（基础输入与文本类 12 个）"; BlockedBy = @("02", "03") },
    @{ Id = "05"; File = ".scratch/winui3-parity-system/issues/05-parity-batch-2-navigation-and-containers.md"; Title = "feat(widgets): 05 接入并校准第二批已实现基准控件（导航、浮层、集合与日期类 23 个）"; BlockedBy = @("04") },
    @{ Id = "06"; File = ".scratch/winui3-parity-system/issues/06-missing-controls-p0-core-inputs-and-pickers.md"; Title = "feat(widgets): 06 实现并校准 WinUI 3 缺失必补 P0 控件（7 个核心输入与日期选择器）"; BlockedBy = @("03") },
    @{ Id = "07"; File = ".scratch/winui3-parity-system/issues/07-missing-controls-p1-p2-collections-and-flyouts.md"; Title = "feat(widgets): 07 实现并校准 WinUI 3 缺失 P1 控件与 Community Toolkit P2 扩展控件"; BlockedBy = @("05", "06") },
    @{ Id = "08"; File = ".scratch/winui3-parity-system/issues/08-canonical-name-contract-migration.md"; Title = "refactor(widgets): 08 完成 WinUI 3 控件规范名迁移的收缩阶段（Contract）"; BlockedBy = @("05", "07") },
    @{ Id = "09"; File = ".scratch/winui3-parity-system/issues/09-unlock-multi-backdrop-parity-matrix.md"; Title = "feat(parity): 09 解锁云母与亚克力等多后景材质下的批量像素差分矩阵"; BlockedBy = @("01", "05") },
    @{ Id = "10"; File = ".scratch/winui3-parity-system/issues/10-finalize-fluent-design-skill-and-audit-exclusive-controls.md"; Title = "feat(widgets): 10 终版固化 Fluent 设计规律技能并全量巡检同化 43 个自有独有控件"; BlockedBy = @("08", "09") }
)

$issueMap = @{}

foreach ($t in $tickets) {
    $rawBody = Get-Content -Raw $t.File
    $blockerRefs = if ($t.BlockedBy.Count -eq 0) {
        "无（可立即开始）"
    } else {
        ($t.BlockedBy | ForEach-Object { "#" + $issueMap[$_] }) -join ", "
    }
    $fullBody = "Part of #$specNum`nBlocked by: $blockerRefs`n`n$rawBody"
    $url = gh issue create `
        --title $t.Title `
        --label "ready-for-agent" `
        --body $fullBody
    $num = ($url -split '/')[-1].Trim()
    $issueMap[$t.Id] = $num
    Write-Host "Created Ticket $($t.Id) -> #$num ($url)" -ForegroundColor Cyan
}

Write-Host "All 1 Spec + 10 Tickets published!" -ForegroundColor Green
