
# 非交互式 Git 规范与静默执行指南

> **核心不变式**：在 AI 智能体自动化运行环境中，Git 操作**严禁弹出任何外部编辑器**（nano、vim、notepad），**严禁挂起等待终端键盘输入**。所有 Git 动作必须静默、非交互且确定性执行。

---

## 1. 环境与配置加固

为防止 Git 在不同操作系统下调用 `$EDITOR` / `$VISUAL`，必须确保应用了以下环境变量或单命令配置：

### 跨平台静默环境变量

| 环境变量 | Windows PowerShell | POSIX Shell (bash / zsh / sh) | 核心用途 |
| :--- | :--- | :--- | :--- |
| `GIT_EDITOR` | `$env:GIT_EDITOR = "cmd.exe /c exit 0"` | `export GIT_EDITOR=true` | 阻止 commit/rebase/merge 调起编辑器 |
| `GIT_SEQUENCE_EDITOR` | `$env:GIT_SEQUENCE_EDITOR = "cmd.exe /c exit 0"` | `export GIT_SEQUENCE_EDITOR=true` | 阻止交互式 rebase TODO 列表弹窗 |
| `GIT_PAGER` | `$env:GIT_PAGER = "cat"` | `export GIT_PAGER=cat` | 阻止分页器（less/more）挂起等待按键 |
| `GIT_TERMINAL_PROMPT` | `$env:GIT_TERMINAL_PROMPT = "0"` | `export GIT_TERMINAL_PROMPT=0` | 阻止凭据/认证提示终端等待输入 |

### 单命令内联注入（最高可靠性推荐）

在 Agent 每次通过 shell 工具执行单条 git 命令时，优先内联注入 `-c core.editor=true`：

```bash
# 通用安全提交流程 / 变基续接 / cherry-pick 续接
git -c core.editor=true commit --amend --no-edit
git -c core.editor=true rebase --continue
git -c core.editor=true cherry-pick --continue
git --no-pager log -n 5 --oneline
```

---

## 2. 命令级防御速查矩阵

| Git 动作 | 易挂起的交互式危险命令 | 安全的非交互式替代方案 | 挂起/弹窗诱因 |
| :--- | :--- | :--- | :--- |
| **提交 (Commit)** | `git commit` (未带 `-m`) | `git commit -m "<type>: <标题>"` | Git 默认打开编辑器等待输入提交信息 |
| **修正 (Amend)** | `git commit --amend` | `git commit --amend --no-edit` | 打开编辑器等待确认修改原信息 |
| **变基续接 (Rebase Continue)** | `git rebase --continue` | `git -c core.editor=true rebase --continue` | 打开编辑器供复核该节点提交说明 |
| **交互变基 (Interactive Rebase)** | `git rebase -i <upstream>` | **在 Agent 流程中严禁使用** | 必然打开编辑器等待 pick/squash 指令 |
| **合并 (Merge)** | `git merge <branch>` | `git merge --no-edit <branch>` | 默认调起编辑器编辑 merge commit 标题 |
| **拉取 (Pull)** | `git pull` (隐式 merge) | `git pull --rebase --autostash` | 变基拉取防无谓 merge 节点与弹窗 |
| **日志与差异 (Log / Diff)** | `git log`, `git diff` | `git --no-pager log`, `git --no-pager diff` | 分页器等待按 'q' 键退出导致进程挂起 |
| **图形合并工具 (Merge Tool)** | `git mergetool` | **严禁调用** | 试图启动外部 GUI 合并器挂起环境 |

---

## 3. 冲突裁决与“零用户手动编辑”红线

1. **Agent 必须全自主就地解决冲突**：
   发生变基或合并冲突时，Agent 拥有完备的工作区文件读写工具（`read_file`, `edit_file`, `write_file`）。
   **Agent 必须自行在代码中定位冲突标记（`<<<<<<<`, `=======`, `>>>>>>>`），结合上下文理解两侧意图，直接修改文件消除冲突，并运行编译/测试校验。**
   **严禁要求用户“请在终端打开编辑器合并冲突”。**

2. **非交互式续接**：
   在解决完冲突标记并使用 `git add <files>` 暂存后：
   - 变基场景：`git -c core.editor=true rebase --continue`
   - 分支同步脚本场景：`pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Continue`

3. **无法裁决时的安全中止与回滚**：
   若冲突涉及互斥的底层架构变动或高风险业务意图冲突，无法安全推断时：
   - 立即执行 `git rebase --abort`（或 `continue-sync.ps1 -Abort`）恢复到干净工作区；
   - 准备清晰的技术报告与明确的方案选项向用户汇报，决不把仓库留在半挂起状态。
