---
name: implement-spec
description: >-
  将 /to-spec 与 /to-tickets 的产出完整实现为代码。
disable-model-invocation: true
---

你拿到了一份 spec。这份 spec 应当已经关联了对应的工单，描述了如何实现它。

issue tracker 的配置应当已经提供给你；如果没有，请提示用户先运行 `/setup-matt-pocock-skills`。

目标是把整份 spec 完整实现到同一条**集成分支（integration branch）**上，并按照 issue tracker 的关闭流程结清每个工单。

这些工单不是线性的步骤清单，而是一张带有阻塞关系的**任务图（task graph）**。这意味着图上始终存在一批已就绪、可立即认领的**前沿（frontier）**工单。

每个 subagent 都必须从全新的上下文窗口启动。

与 subagent 之间的通信应当精简。主要通过**上下文指针（context pointers）**通信：指向 spec、工单、调研笔记和先前的提交记录。不要重复复制通过指针即可读取的信息。

**实现者 subagent（Implementer subagents）**应尽量在后台并发运行，以实现最大并发度。

## 步骤

1. 阅读 spec 与工单，理清任务图依赖关系。

2. （可选）使用一个**探索 subagent** 完成工单所需的背景探索——相关代码文件或外部文档。确保探索 subagent 能够落盘文件：它应把 Markdown 笔记保存在仓库外、且所有后续 subagent 都能访问的目录中。这样**实现者 subagent**就能专注于实现，而不必重复探索。

3. 创建集成分支。如果 issue tracker 是通过 PR 关闭工单的，或者用户明确要求开 PR，在第 5 步完成第一次合并后立即打开一个草稿 PR（还没有领先 main 的提交时无法创建 PR），并标记为关闭对应的 spec 与工单。

4. 为每个工单拉起一个全新的**实现者 subagent**，各自运行在独立的 worktree 与独立分支上。每个实现者 subagent 需做到：
   - 动工前确认自己的 worktree 基于当前集成分支创建，若不是则 reset 到集成分支；
   - 调用 Skill 工具执行 `tdd` 来完成该工单；
   - 在汇报完成前，先把集成分支最新 tip 合并回自己的分支。

5. 一旦某个**实现者 subagent** 完成，通过一个**合并者 subagent（merger subagent）**将其工作合并到集成分支。

6. 如果这次合并推进了可用工单的**前沿**，立刻拉起更多**实现者 subagent** 处理新解锁的工单，保持最大并发。

7. 所有工单完成后，在集成分支上调用 Skill 工具执行 `code-review`。用一个全新的**实现者 subagent** 集中修复 code review 提出的所有问题。

8. 如果存在草稿 PR，将其标记为 Ready for Review；否则按 issue tracker 的关闭方式逐一关闭工单，并汇报集成分支情况。

9. 清理所有**实现者 subagent** 的 worktree。