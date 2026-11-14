
# 变基 (Rebase) 与冲突自主裁决指南

确定性非交互式变基、代码级全自主冲突清理与安全回滚恢复规范。

---

## 1. 静默变基最佳实践

变基操作必须严格处于非交互模式下运行，严禁调用 `git rebase -i` 或诱发 Git 调起外部编辑器。

### 安全变基命令集

```bash
# 线性拉取远端更新，避免产生无谓 merge 节点
git pull --rebase --autostash

# 将当前分支变基到目标分支
git rebase <target-branch>

# 当变基遇到冲突暂停时，在代码中解决并暂存后，静默续接：
git add <resolved-files>
git -c core.editor=true rebase --continue
```

---

## 2. 代码级全自主冲突裁决（严禁转交用户编辑）

当变基或合流因代码冲突暂停时，严格执行以下四步闭环：

### 第 1 步：检索冲突文件清单
```bash
git status --short
# 标记为 UU 或双方修改的文件即为冲突源
```

### 第 2 步：定位冲突标记
使用文件读取工具查看冲突文件，识别标准冲突区间结构：

```text
<<<<<<< HEAD (集成分支或当前基线的既有代码)
existing_code_block();
=======
incoming_feature_code_block();
>>>>>>> commit-hash (正在合入的提交代码)
```

### 第 3 步：自主合并与清理
- 结合上下文及两侧提交信息，分析业务逻辑；
- 在工作区使用代码编辑工具（`edit_file` / `write_file`）就地完成代码融合；
- **彻底清除所有冲突标记行（`<<<<<<<`, `=======`, `>>>>>>>`）**；
- **硬性红线**：严禁向用户要求“请您在终端打开编辑器合并冲突”。

### 第 4 步：构建验证与静默续接
- 运行针对性编译或测试命令验证无语法错误；
- 暂存已解决文件：
  ```bash
  git add <resolved-files>
  ```
- 静默续接执行：
  - 原生变基场景：
    ```bash
    git -c core.editor=true rebase --continue
    ```
  - `sync-branch.ps1` 同步管线场景：
    ```powershell
    pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Continue
    ```

---

## 3. 安全中止与决策上报

若冲突涉及互斥的底层架构变动或高风险业务意图冲突，无法通过代码分析安全裁决时：
1. **绝不将 Git 留在半变基的悬挂状态**；
2. 立即安全中止回滚：
   ```bash
   git rebase --abort
   # 或在同步管线中执行：
   pwsh .agents/skills/git-workflow/scripts/continue-sync.ps1 -Abort
   ```
3. 恢复干净工作区后，向用户提供清晰的冲突场景分析与决策选项，由用户决定策略后再执行。
