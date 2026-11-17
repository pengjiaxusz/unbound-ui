# Issue 跟踪器：GitHub

本仓库的 issue 与 spec 存放在 GitHub Issues 中，所有操作使用 `gh` CLI。

## 约定

- **创建 issue**：`gh issue create --title "..." --body "..."`。多行正文使用 heredoc。
- **读取 issue**：`gh issue view <number> --json number,title,body,labels,comments`。
- **列出 issue**：`gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`，按需搭配 `--label` 与 `--state` 过滤。
- **设为父 issue 的子 issue**：`gh issue create --parent <parent> ...`，或事后执行 `gh issue edit <parent> --add-sub-issue <child>`（`gh` 2.94+）。旧版 `gh` 用：`gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`（数据库 ID，见下文**阻塞关系**）。若平台不支持 sub-issues，在子 issue 正文顶部写 `Part of #<parent>`。
- **阻塞关系（Blocking）**：使用 GitHub **原生 issue 依赖关系**，这是 UI 可见的权威表达。添加依赖边：`gh issue create --blocked-by <n>,<n> ...` 或 `gh issue edit <child> --add-blocked-by <blocker>`（`gh` 2.94+）。旧版 `gh`：`gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>`，其中 `<blocker-db-id>` 是阻塞者的数字**数据库 ID**（`gh api repos/<owner>/<repo>/issues/<n> --jq .id`，*不是* `#number` 或 `node_id`）。若原生依赖不可用，退回到在子 issue 正文顶部写一行 `Blocked by: #<n>, #<n>`。当所有阻塞者均已关闭时，工单解除阻塞。
- **统计未关闭的阻塞者数量**：`gh api repos/<owner>/<repo>/issues/<n> --jq .issue_dependencies_summary.blocked_by`（仅统计 open 阻塞者；为 0 表示可开工）。`gh issue view <n> --json blockedBy` 也会列出已关闭的阻塞者，需过滤：`--jq '[.blockedBy.nodes[] | select(.state == "OPEN")] | length'`。
- **评论 issue**：`gh issue comment <number> --body "..."`
- **添加 / 移除标签**：`gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **关闭 issue**：先用 `gh issue comment <number> --body "..."` 发说明，再执行 `gh issue close <number>`。对于已关闭的 issue，`gh issue close --comment` 会丢弃评论并仍返回 0。

通过 `git remote -v` 推断仓库；在克隆目录中运行 `gh` 时会自动识别。

只向用户自己的仓库提交 issue。除非用户是仓库所有者，否则绝不要把 issue 发到 `mattpocock/skills`（本套技能来源仓库）。在 fork 仓库中，`gh` 可能默认指向游父仓库，因此在每次会话首次执行 `gh issue create` 前，先用 `gh repo view --json nameWithOwner` 检查目标，若不是用户自己的仓库，先向用户确认。

## 将 Pull Request 作为分诊入口

**PRs as a request surface: no.** _（若本仓库把外部 PR 当作特性请求处理，改为 `yes`；`/triage` 会读取此标记。）_

设为 `yes` 时，PR 与 issue 走相同的标签与状态流转，使用对应的 `gh pr` 命令：

- **读取 PR**：`gh pr view <number>`，再 `gh pr view <number> --comments`（非 TTY 下 `--comments` 只打印评论不打印正文），以及 `gh pr diff <number>` 查看 diff。
- **列出待分诊的外部 PR**：`gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`。
- **评论 / 打标签 / 关闭**：`gh pr comment`、`gh pr edit --add-label`/`--remove-label`、`gh pr close`。

GitHub 的 issue 与 PR 共用同一套编号空间，因此裸写的 `#42` 可能是其中任意一种：先用 `gh pr view 42` 解析，失败再回退到 `gh issue view 42`。

## 当技能说“发布到 issue tracker”时

创建一个 GitHub issue。

## 当技能说“拉取相关工单”时

按上文**读取 issue** 的方式读取。

## Wayfinding 操作

供 `/wayfinder` 使用。**地图（map）**是一个单独的 issue，**子 issue** 作为各决策工单。

- **地图**：带 `wayfinder:map` 标签的单个 issue，正文包含 Notes / Decisions-so-far / Fog。命令：`gh issue create --label wayfinder:map`。
- **子工单**：作为 GitHub sub-issue 关联到地图的 issue（见**设为父 issue 的子 issue**）。若未开启 sub-issues，把子工单加入地图正文的任务列表，并在子工单正文顶部写 `Part of #<map>`。标签为 `wayfinder:<type>`（`research`/`prototype`/`grilling`/`task`）。认领后指派给推动该任务的开发者。
- **阻塞关系**：见上文约定中的**阻塞关系**。
- **前沿查询**：列出地图下所有 open 子工单（`gh issue list --state open`，限定在地图的 sub-issues / 任务列表），排除仍有未关闭阻塞者（见**统计未关闭的阻塞者数量**，或 `Blocked by` 行中仍有 open issue）或已有指派人的工单；按地图顺序取第一个。
- **认领**：`gh issue edit <n> --add-assignee @me`，作为会话的第一次写操作。
- **解决**：`gh issue comment <n> --body "<answer>"`，然后 `gh issue close <n>`，再向地图的 Decisions-so-far 追加一条上下文指针（要点摘要 + 链接）。