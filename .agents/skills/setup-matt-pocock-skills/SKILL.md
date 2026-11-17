---
name: setup-matt-pocock-skills
description: >-
  为当前仓库配置工程类技能所需的环境：设置 issue tracker、分诊标签词汇表以及领域文档布局。在首次使用其他工程类技能前运行一次。
disable-model-invocation: true
---

# 配置 Matt Pocock 工程技能环境

为本仓库搭建其他工程技能所依赖的项目级配置：

- **Issue tracker**：issue 存放在哪里（默认 GitHub；也开箱支持本地 Markdown）
- **分诊标签（Triage labels）**：五个规范分诊角色对应的实际标签名
- **领域文档（Domain docs）**：`GLOSSARY.md` 与 ADR 的存放位置，以及读取它们的消费规则

这是一个由提示词驱动的技能，而不是确定性脚本。先探索、汇报发现、与用户确认，然后再写入。

## 流程

### 1. 探索

查看当前仓库以了解初始状态。有什么读什么，不要靠猜：

- `git remote -v` 与 `.git/config`：这是一个 GitHub 仓库吗？指向哪一个？
- 仓库根目录的 `AGENTS.md` 与 `CLAUDE.md`：是否存在？里面是否已经有 `## Agent skills` 小节？
- 仓库根目录的 `GLOSSARY.md` 与 `GLOSSARY-MAP.md`，以及旧版的 `CONTEXT.md` / `CONTEXT-MAP.md`。如果存在旧版文件，提议用 `git mv` 重命名为 `GLOSSARY` 名称，并在 `docs/agents/domain.md` 中同步改名。
- `docs/adr/` 与任何 `src/*/docs/adr/` 目录
- `docs/agents/`：本技能之前是否已经生成过输出？
- `.scratch/`：是否已经采用了本地 Markdown issue tracker 约定？
- 是否安装了 `triage` 技能？（旁边存在 `triage` 技能目录，或可用技能中有 `triage`。）这决定了 B 部分是否需要执行。
- Monorepo 信号：`pnpm-workspace.yaml`、`package.json` 中的 `workspaces` 字段，或包含各自 `src/` 的 `packages/*`。只有真正大型的多包仓库才有这些信号；没有则意味着单上下文，几乎所有仓库都是如此。

### 2. 呈现发现并逐项确认

总结已存在什么、缺少什么。然后按顺序推进各部分：一个部分确认完，再进下一个。

每个部分都把你的推荐答案放在最前面，让用户回一个词就能采纳。只在确实存在分支选择时给出一行解释；探索阶段已经定案的部分直接跳过（没装 `triage` 时跳过 B 部分，非 monorepo 时跳过 C 部分）。

**A 部分：Issue tracker。**

> 说明：“issue tracker”是本仓库存放 issue 的地方。`to-tickets`、`triage`、`to-spec` 等技能都会读写它。它们需要知道是调用 `gh issue create`、在 `.scratch/` 下写 Markdown 文件，还是遵循你描述的其他工作流。选择你实际用来跟踪本仓库工作的位置。

默认姿态：这套技能最初为 GitHub 设计。如果 `git remote` 指向 GitHub，推荐 GitHub；如果指向 GitLab（`gitlab.com` 或自建实例），推荐 GitLab。否则（或用户偏好其他方式），提供以下选项：

- **GitHub**：issue 存放在仓库的 GitHub Issues（使用 `gh` CLI）
- **GitLab**：issue 存放在仓库的 GitLab Issues（使用 [`glab`](https://gitlab.com/gitlab-org/cli) CLI）
- **本地 Markdown**：issue 作为文件存放在本仓库 `.scratch/<feature>/` 下（适合个人项目或无远端仓库）
- **其他**（Jira、Linear 等）：请用户用一段话描述工作流，本技能将原样记录为自由文本说明

将选择记录到 `docs/agents/issue-tracker.md`。GitHub 与 GitLab 模板中带有一个“将 PR/MR 作为请求入口”的开关，默认**关闭（no）**。保持关闭且无需主动提起：如果用户日后想把外部 PR 纳入分诊队列，自己去改该文件里的开关即可。

选择 GitHub 或 GitLab tracker 需要一个可触达的仓库。如果用户选了其中之一但当前没有指向该平台的 `git remote`，先向用户要到仓库 URL 并配置好（必要时 `git init`、`git remote add origin <url>`）再继续。当在该目录下运行 `gh repo view` / `glab repo view` 能打印项目信息时才算就绪；如果不行，停下来告诉用户缺什么。

**B 部分：分诊标签词汇表。** 如果未安装 `triage` 技能（探索阶段已确认），完全跳过本部分——没装的技能不需要标签。

如果已安装，只问这一个问题：

> 要保留默认的分诊标签吗？（推荐：**是**）

默认即五个规范角色，标签字符串与角色名完全一致：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。用户回答**是**时原样写入。只有当用户说不（通常因为他们的 tracker 已经在使用其他命名，例如用 `bug:triage` 对应 `needs-triage`）时，才收集覆盖映射，让 `triage` 复用既有标签而不是新建重复标签。

**C 部分：领域文档布局。** 默认采用**单上下文**（仓库根目录一个 `GLOSSARY.md` + `docs/adr/`）。这适合几乎所有仓库，无需询问直接写入。

仅当探索阶段发现了 monorepo 信号时，才提供**多上下文**选项（根目录 `GLOSSARY-MAP.md` 指向各上下文自己的 `GLOSSARY.md`），并让用户确认布局。

### 3. 展示草稿并确认

向用户展示以下内容的草稿：

- 准备写入 `CLAUDE.md` / `AGENTS.md`（选择规则见第 4 步）的 `## Agent skills` 代码块
- `docs/agents/issue-tracker.md`、`docs/agents/domain.md` 以及（仅当安装了 `triage` 时）`docs/agents/triage-labels.md` 的内容

写入前允许用户修改。

### 4. 写入

**选择要编辑的文件：**

- 若 `CLAUDE.md` 存在，编辑它。
- 否则若 `AGENTS.md` 存在，编辑它。
- 若两者都不存在，询问用户要创建哪一个，不要替他选。

绝不要在 `CLAUDE.md` 已存在时再去创建 `AGENTS.md`（反之亦然）；始终编辑已经存在的那一个。

如果所选文件中已经存在 `## Agent skills` 区块，就地更新其内容，不要追加重复区块，也不要覆盖用户对周边章节的编辑。

写入的区块格式：

```markdown
## Agent skills

### Issue tracker

[用一行概括 issue 存放在何处]。详见 `docs/agents/issue-tracker.md`。

### Triage labels

[用一行概括标签词汇表]。详见 `docs/agents/triage-labels.md`。

### Domain docs

[用一行概括布局："single-context" 或 "multi-context"]。详见 `docs/agents/domain.md`。
```

仅当安装了 `triage` 且执行了 B 部分时，才包含 `### Triage labels` 子块并写入 `docs/agents/triage-labels.md`；否则两者均省略。

当在 GitHub 或 GitLab 上执行了 B 部分时，创建 tracker 中尚未存在的已配置标签（`gh label create` / `glab label create`）。

然后以本技能目录下的种子模板为起点写入文档文件：

- [issue-tracker-github.md](./issue-tracker-github.md)：GitHub issue tracker
- [issue-tracker-gitlab.md](./issue-tracker-gitlab.md)：GitLab issue tracker
- [issue-tracker-local.md](./issue-tracker-local.md)：本地 Markdown issue tracker
- [triage-labels.md](./triage-labels.md)：标签映射（仅当安装了 `triage` 时）
- [domain.md](./domain.md)：领域文档消费规则与布局

对于“其他” issue tracker，直接根据用户的描述从零编写 `docs/agents/issue-tracker.md`。

### 5. 完成

告知用户配置已完成，并说明哪些工程技能现在会读取这些文件。提醒用户日后可以直接手动编辑 `docs/agents/*.md`；只有需要切换 issue tracker 或推倒重来时才需要重新运行本技能。

> **模板兼容说明（`pengj-templates`）：** 若仓库的 `AGENTS.md` 包含 `<!-- PENGJ_TEMPLATE_START --> ... <!-- PENGJ_TEMPLATE_END -->` 托管块，务必将 `## Agent skills` 区块写在托管块**之外**（`<!-- PENGJ_TEMPLATE_END -->` 之后），以免在执行 `pengj-templates update` 时被模板更新覆盖。