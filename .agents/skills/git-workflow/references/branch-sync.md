
# 分支同步与线性合流深度规范

基于 Worktree 感知的极速线性分支同步、持久化快照保护、时间序合入与 Tree-Diff Guard 改动保全机制。

---

## 1. 1-Shot 线性同步合流管线

在多分支合并、Worktree 协同与主干对齐场景下，直接运行一键合流脚本：

```powershell
# 1-Shot 闭环同步所有分支到当前工作目录激活的分支
pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -Apply

# 同步指定单一分支
pwsh .agents/skills/git-workflow/scripts/sync-branch.ps1 -SourceBranch 'feat/x' -Apply

# 只读拓扑巡检（零副作用查看所有分支 Ahead / Behind / Net 提交）
pwsh .agents/skills/git-workflow/scripts/show-branch-topology.ps1 -Detailed
```

### 自动化同步管线步骤

1. **动态集成分支解析**：默认以当前工作区激活分支作为集成分支；
2. **远端真值探测**：直接走 `git ls-remote` 获取权威指针，避免信赖陈旧的 `refs/remotes/*`；
3. **净贡献提取与时间序排序**：通过 `rev-list --not` 与 patch-id 去重，按提交时间戳（`%ct`）升序全局排序；
4. **安全快照沉淀**：变动前为所有分支建立 `refs/sync-backup/<branch>/<timestamp>-<sha>` 本地持久指针；
5. **纯线性合流**：优先 fast-forward / 衍合或有序 cherry-pick，严禁产生 merge commit；
6. **Tree-Diff Guard 零丢改动校验**：双重校验确保源分支的所有代码变动 100% 完整合入；
7. **批量 Worktree 与分支对齐**：通过 safe push（`--force-with-lease`）安全推送到远端并重置所有关联 worktree；
8. **自动化核验与状态看板**：自动执行项目定义的测试验证套件。

---

## 2. Tree-Diff Guard 树级防丢机制

在重置与推送前，必须通过双重校验：
- **无残留提交**：`git cherry -v` 的 `+` 标识必须清零；
- **文件树完整性**：源分支自 merge-base 以来变动的所有文件，在集成分支中必须全部如实体现。

**任一检查不通过，绝对不会重置分支或推送到远端**，并自动安全回滚到合流前快照。

---

## 3. 灾备恢复与快照管理

若合流遇到异常或需要回溯历史分支：

```powershell
# 查看本地所有的安全快照
pwsh .agents/skills/git-workflow/scripts/manage-sync-backups.ps1 -List

# 还原指定分支到指定快照指针
pwsh .agents/skills/git-workflow/scripts/manage-sync-backups.ps1 -RestoreBranch 'feat/x' -BackupRef 'refs/sync-backup/...'
```
