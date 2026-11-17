# Issue 跟踪器：GitLab

本仓库的 issue 与 spec 存放在 GitLab Issues 中，所有操作使用 [`glab`](https://gitlab.com/gitlab-org/cli) CLI。

## 约定

- **创建 issue**：`glab issue create --title "..." --description "..."`。多行描述使用 heredoc，传 `--description -` 可打开编辑器。
- **读取 issue**：`glab issue view <number> --comments`。需要机器可读输出时加 `-F json`。
- **列出 issue**：`glab issue list -O json`，按需搭配 `--label` 过滤。
- **评论 issue**：`glab issue note <number> --message "..."`。GitLab 中把评论称作 "notes"。
- **添加 / 移除标签**：`glab issue update <number> --label "..."` / `--unlabel "..."`。多个标签可用逗号分隔或重复传入参数。
- **关闭 issue**：`glab issue close <number>`。`glab issue close` 不支持附带关闭说明，请先用 `glab issue note <number> --message "..."` 发说明，再关闭。
- **Merge requests**：GitLab 将 PR 称作 "merge requests"。使用 `glab mr create`、`glab mr view`、`glab mr note` 等，形态与 `gh pr ...` 相同，只需把 `pr` 换成 `mr`、把 `comment`/`--body` 换成 `note`/`--message`。

通过 `git remote -v` 推断仓库；在克隆目录中运行 `glab` 时会自动识别。

## 将 Merge Request 作为分诊入口

**MRs as a request surface: no.** _（若本仓库把外部 MR 当作特性请求处理，改为 `yes`；`/triage` 会读取此标记。）_

设为 `yes` 时，MR 与 issue 走相同的标签与状态流转，使用对应的 `glab mr` 命令：

- **读取 MR**：`glab mr view <number> --comments`，以及 `glab mr diff <number>` 查看 diff。
- **列出待分诊的外部 MR**：`glab mr list -O json`，仅保留作者不是项目成员/所有者的 MR（社区贡献者的 MR，而非维护者进行中的工作）。
- **评论 / 打标签 / 关闭**：`glab mr note`、`glab mr update --label`/`--unlabel`、`glab mr close`。

与 GitHub 不同，GitLab 的 issue 与 MR 各自独立编号，因此只要明确维护者指的是哪种入口，`#42` 就不会产生歧义。

## 当技能说“发布到 issue tracker”时

创建一个 GitLab issue。

## 当技能说“拉取相关工单”时

运行 `glab issue view <number> --comments`。

## Wayfinding 操作

供 `/wayfinder` 使用。**地图（map）**是一个单独的 issue，**子 issue** 作为各决策工单。

- **地图**：带 `wayfinder:map` 标签的单个 issue，正文包含 Notes / Decisions-so-far / Fog。命令：`glab issue create --label wayfinder:map`。（在支持原生 Epic 的 GitLab 版本中也可由 Epic 承载，但带标签的 issue 通用性最好。）
- **子工单**：在描述顶部写 `Part of #<map>`、关联到地图（`glab issue create --linked-issues <map> --link-type relates_to`）并带标签 `wayfinder:<type>`（`research`/`prototype`/`grilling`/`task`）的 issue。认领后指派给推动该任务的开发者。
- **阻塞关系（Blocking）**：使用 GitLab **原生阻塞链接**，这是 UI 可见的权威表达。通过快捷指令 note 添加：`/blocked_by #<n>`（`glab issue note <child> --message "/blocked_by #<blocker>"`）。原生阻塞链接属 Premium/Ultimate 特性；在免费版（或不可用时）退回到在描述顶部写一行 `Blocked by: #<n>, #<n>`。当所有阻塞者均已关闭时，工单解除阻塞。
- **前沿查询**：用 `glab api projects/:id/issues/<map>/links` 列出地图子项，保留带有 `wayfinder:<type>` 标签的 open issue；排除仍有未关闭阻塞者（指向 open issue 的原生 `blocked_by` 链接 `glab api projects/:id/issues/<child-iid>/links`，或 `Blocked by` 行中的 open issue）或已有指派人的工单；按地图顺序取第一个。
- **认领**：`glab issue update <n> --assignee @me`，作为会话的第一次写操作。
- **解决**：`glab issue note <n> --message "<answer>"`，然后 `glab issue close <n>`，再向地图的 Decisions-so-far 追加一条上下文指针（要点摘要 + 链接）。