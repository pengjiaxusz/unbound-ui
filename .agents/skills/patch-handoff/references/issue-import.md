
# 交接包的 issue 导入，以及落地后的收口

交接包带工单的频率和带补丁差不多，而 zip 里的工单等于没人能点开的工单。两个时刻：落补丁前导入，推送后收口。

## 一、包里可能带的几种形态

| 形态 | 长什么样 | 怎么导入 |
| :--- | :--- | :--- |
| issue 包 | `issues/NNNN-*.md`，front matter 带 `key` / `title` / `labels` / `parent` / `blocked_by` | `issue-push.ps1` —— 与 `issue-sync` 的导出/推送同一套格式 |
| 文档里的表 | 一张 markdown 表：补丁 → 工单 → 标题 | 每行转成一个带 `key` 的 issue 文件，再推整个目录 |
| 只有编号 | 文档直接写 `#102–#113` | 无需导入；逐个 `gh issue view <n>` 核实并沿用编号 |

优先用 bundle 形态：它带着 `parent` 与 `blocked_by` 的依赖接线，而手写 `gh issue create` 循环会把这些关系丢掉。

## 二、导入前三个检查

1. **目标仓库。** `gh repo view --json nameWithOwner` —— fork 里 `gh` 默认打上游仓库。显式传 `-Repo <owner/name>` 并核对。
2. **标签存在。** front matter 里写了仓库没有的标签，整批会失败。先 `gh label list`；缺的标签要么有意创建，要么从文件里去掉并在报告里说明。
3. **重复。** 文档引用的编号已存在就别重建。若这批之前导过一次，脚本按 `key` 幂等——但**重新生成过的 key**（`T-01` 变成 `S1-01`）会造出重复单；推之前先 `gh issue list --search '"<标题>"'`。

## 三、导入

```powershell
# 预演：打印每次创建、父子链接与 blocked_by 边，不写远端
pwsh .agents/skills/issue-sync/scripts/issue-push.ps1 -Repo <owner/name> `
    -InputDir <pkg>/issues -Mapping ./issue-mapping.json -DryRun

# 真建；撞到时间预算就加 -Resume 续跑
pwsh .agents/skills/issue-sync/scripts/issue-push.ps1 -Repo <owner/name> `
    -InputDir <pkg>/issues -Mapping ./issue-mapping.json
```

它产出 `issue-mapping.json`（`key` → 真实编号），把 `number` / `url` 回填进 issue 文件，并把 parent 挂成 sub-issue、依赖连成原生 `blocked_by`。

**项目里没有 `issue-sync` 技能**（它不是基线技能）：按依赖顺序逐条 `gh issue create --title ... --body-file ...`，再按同样的形状手工记映射。契约相同，只是慢。

## 四、映射表

关任何单之前，先把这张表写进报告：

```
补丁    工单     #     是否落地
0001    T-01    #101  是（3f2a1c9）
0004    T-04    #104  否（已丢弃：#101 已覆盖）
```

取值优先级：文档自己的 补丁↔工单 表 → issue 标题里已写明的编号 → `issue-mapping.json` 的 key。只按显式标识匹配（补丁标题里嵌的 key，或文档明写的编号）。**补丁与工单对不上就停手问用户。** 猜一个编号，关掉的是别人的工单。

## 五、按映射收口

| 状态 | 动作 |
| :--- | :--- |
| 已落地、折进提交 `abc1234` | `gh issue comment <n>`（SHA、门禁、通过数）→ `gh issue close <n>` → 标签：先加新、再撤旧 |
| 补丁丢弃 / 折到别处 | 保持 **open**，评论写清是哪条补丁、为什么 |
| 交接前就已关闭 | 保持关闭，只补一条落地评论，不要重新打开 |

`gh issue close <n> --comment "..."` 对已关闭的 issue 会丢评论并仍返回 0 —— 先评论、再关闭、然后读输出。

## 六、失败模式

- **标签缺失** —— 批次中途中断，有的单建了有的没建。补好标签后重跑，脚本按 `key` 续上。
- **`gh` 低于 2.94** —— `gh issue` 没有子工单与依赖的子命令，这两类边要走 `gh api` 的 REST 端点，脚本里已经这么做了；不要为了绕开它去手写 GraphQL。
- **撞时间预算** —— 状态文件已落盘，加 `-Resume` 续跑，不要从头再来。
- **`gh` 未登录** —— 报告必须写 `issue 未导入：gh 未登录` **外加原样命令**，不许把工单清单改写成一段文字然后声称已跟踪。
- **编号来自别的仓库** —— 对着上游项目写的文档引的是上游编号，在你的 fork 里毫无意义。重新导入拿新编号，并在报告里说明。

## 七、S8 要删的临时产物

`issue-mapping.json`、解包出来的 `issues/` 目录、解包目录本身、`fix-spec/`、日志。列清单、确认、删除——然后验证解包目录已不存在且 `git status --porcelain -uall` 为空。
