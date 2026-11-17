# Issue 跟踪器：本地 Markdown

本仓库的 issue 与 spec 以 Markdown 文件形式存放在 `.scratch/` 下。

## 约定

- 每个特性一个目录：`.scratch/<feature-slug>/`
- spec 为 `.scratch/<feature-slug>/spec.md`
- 实现工单为每个工单一个文件：`.scratch/<feature-slug>/issues/<NN>-<slug>.md`，从 `01` 开始编号，绝不把所有工单塞进同一个文件
- 分诊状态记录在每个工单文件顶部附近的 `Status:` 行（角色字符串见 `triage-labels.md`）
- 评论与讨论历史追加在文件底部的 `## Comments` 标题下

## 当技能说“发布到 issue tracker”时

在 `.scratch/<feature-slug>/` 下创建新文件（必要时先创建目录）。

## 当技能说“拉取相关工单”时

读取所引用路径的文件。用户通常会直接提供路径或工单编号。

## Wayfinding 操作

供 `/wayfinder` 使用。**地图（map）**是一个文件，每个子工单对应一个独立文件。

- **地图**：`.scratch/<effort>/map.md`（包含 Notes / Decisions-so-far / Fog）。
- **子工单**：`.scratch/<effort>/issues/NN-<slug>.md`，从 `01` 开始编号，正文写待解决的问题。`Type:` 行记录工单类型（`research`/`prototype`/`grilling`/`task`）；`Status:` 行记录 `claimed`/`resolved`。
- **阻塞关系**：顶部附近写一行 `Blocked by: NN, NN`。当列出的每一个文件都变为 `resolved` 时，该工单解除阻塞。
- **前沿查询**：扫描 `.scratch/<effort>/issues/` 中状态既不是 `resolved` 也未被 `claimed`、且无未解决阻塞者的文件；按编号取第一个。
- **认领**：在开始任何工作前先设置 `Status: claimed` 并保存。
- **解决**：在 `## Answer` 标题下追加答案，将状态设为 `Status: resolved`，然后向 `map.md` 的 Decisions-so-far 追加一条上下文指针（要点摘要 + 链接）。