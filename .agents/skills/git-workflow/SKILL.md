---
name: git-workflow
description: >-

  通用 Git 操作与工作流自动化技能：包含约定式提交、Worktree 感知的极速线性分支同步、非交互变基与代码级冲突自主裁决。核心公用规则严格杜绝外部编辑器弹窗与挂起（GIT_EDITOR=true、--no-edit、严禁让用户手动编辑）。通过渐进式展开路由到提交、分支同步与变基子功能。
  Triggers: git, git-workflow, commit, branch-sync, rebase, cherry-pick, merge, push, pull, 提交, 变基, 衍合, 同步分支, 分支同步, 合并分支, 合入主分支, worktree 同步.

---

<!-- PENGJ_TEMPLATE_START -->

# Git Workflow — 通用 Git 操作与渐进式工作流规范

统管约定式提交、Worktree 线性分支同步、非交互式变基与代码级冲突自主裁决的通用 Git 技能。

---

## ⚡ 0. 核心公用规则：全局非交互铁律（杜绝用户手动编辑）

在 AI 智能体自动化运行环境中，Git 操作**严禁弹出任何外部编辑器**（nano、vim、notepad），**严禁挂起等待终端键盘输入**：
1. **环境静默加固**：执行环境统一注入 `$env:GIT_EDITOR = "cmd.exe /c exit 0"`（Windows pwsh）或 `GIT_EDITOR=true`（POSIX shell）、`GIT_SEQUENCE_EDITOR=true` 与 `GIT_PAGER=cat`；
2. **命令参数约束**：提交严格带 `-m "<标题>"`，修正/合并必须带 `--no-edit`，变基续接强制加 `-c core.editor=true`；
3. **Agent 自主解决冲突**：发生变基或合流冲突时，Agent 拥有文件读写能力，必须就地在代码文件中查找并清除冲突标记（`<<<<<<<`），自主完成逻辑融合。**严禁要求用户手动打开外部编辑器合并**；
4. **无法安全裁决时主动回滚**：若代码冲突涉及互斥架构变动无法安全推断，立即执行 `git rebase --abort`（或调用恢复脚本）恢复干净工作区，再向用户汇报决策选项，决不留下半变基挂起状态。

完整环境配置与命令矩阵：[references/non-interactive.md](references/non-interactive.md)。

---

## 🚀 1. 子功能：约定式提交与完工推送 (Commit & Push)

提交后立即 push，防止工作丢失。

### 1.1 触发时机（主动触发规则）
- **代码修改与阶段性完工（回复前提交门禁（硬性拦截，禁止堆积））**：凡本轮对话有任何代码/文档文件的创建、修改或删除，且通过了构建与测试验证，**在停止工具调用并向用户输出最终回复前，必须将运行提交流程（完整性检查、语义拆分、commit 与 `git push`）作为最后一个工具调用步骤**。严禁在工作区留存已验证的代码修改就结束工具调用向用户汇报；严禁在多轮对话中拖延堆积改动或等待用户催促 `/commit`。
  - **规划模式（`/plan` / `/boost`）特别约束**：即使在规划模式下生成或更新了 `walkthrough.md`，写完文档亦绝不等于任务完结；**严禁仅写完 walkthrough 就直接交差，必须紧随其后执行 commit 与 push，未提交推送前严禁结束对话汇报**。
  - **无需提交的豁免情况**：仅当本轮对话为纯问答/无任何文件变动，或构建/测试失败正在排查取证阶段且需向用户提问时，方可不提交。
- **任务收尾要求**：任何开发或维护任务结束、向用户做最终交差前，工作区必须保持干净，所有改动必须已提交并推送到远端（commit & push）。
- **用户显式指令**：用户发送 `/commit`、"提交"、"commit & push" 等。

### 1.2 提交前完整性检查 —— 项目自定义
本步骤归**项目**所有，模板不硬编码具体检查：按下方托管块外「项目专属提交流程与红线」的定义执行。项目专属区未定义时执行兜底自检（扫 diff 校验构建/更新文档/跑格式化）。
改动涉及模块边界、公共 API、数据流、生命周期或新增模块/子系统时，同步更新对应的架构/设计文档再提交。
- **Rust 静态检查**：运行 `just check`（或 `cargo clippy --all-targets --all-features -- -D warnings`，工作区加 `--workspace`），warnings 清零后再提交。

### 1.3 提交格式与立即推送
执行 `git commit -m "<type>(<scope>): <标题>"` 后立即 `git push`。

- 标题用中文短句（至少包含 2 个汉字，严禁写纯英文标题）；
- type/scope 用英文（`feat` `fix` `docs` `style` `refactor` `perf` `test` `build` `ci` `chore` `revert`）。


提交规范、语义拆分与 Amend 约束详见：[references/commit.md](references/commit.md)。

---

## 🔄 2. 子功能：Worktree 感知的高效线性分支同步 (Branch Sync)

默认以当前工作目录激活分支为底，**1-Shot** 闭环同步所有分支或指定特性分支，内置安全快照（`refs/sync-backup/`）、全局时间序与 Tree-Diff Guard 零丢改动校验：

```powershell
# 推荐：1-Shot 闭环同步所有分支到当前工作目录激活的分支
pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -Apply

# 同步指定单一特性分支
pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -SourceBranch 'feat/x' -Apply

# 只读拓扑巡检（零副作用查看所有分支 Ahead / Behind / Net 提交）
pwsh .agents/skills/git-workflow/scripts/show-branch-topology.ps1 -Detailed
```

分支同步原理、Tree-Diff Guard 与灾备恢复：[references/branch-sync.md](references/branch-sync.md)。

---

## 🔀 3. 子功能：变基与冲突自主裁决 (Rebase & Conflict)

平滑变基同步远端最新改动，杜绝污染分支历史：

```bash
# 线性拉取远端更新
git pull --rebase --autostash

# 若发生变基冲突：在代码中清除冲突标记并暂存后，静默续接：
git add <resolved-files>
git -c core.editor=true rebase --continue
# 或在 sync-branch 管线中续接：
pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Continue

# 无法安全裁决时立即安全中止：
git rebase --abort
# 或在 sync-branch 管线中中止：
pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Abort
```

变基步骤、冲突标记识别与安全回滚详见：[references/rebase-conflict.md](references/rebase-conflict.md)。

---

## 📚 4. 渐进式展开参考索引

- [非交互式 Git 规范与加固清单](references/non-interactive.md)
- [约定式提交规范与门禁](references/commit.md)
- [分支同步与 Tree-Diff Guard 机制](references/branch-sync.md)
- [变基与冲突自主裁决指南](references/rebase-conflict.md)

<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属区域默认留空（在此添加项目自定义的提交前检查与专属门禁） -->
