---
name: handoff
description: >-
  将当前对话精简整理为交接文档，以便另一个 Agent 接手继续。
argument-hint: "What will the next session be used for?"
disable-model-invocation: true
---

撰写一份交接文档，总结当前对话以便新的 Agent 能够无缝继续工作。将其保存到用户操作系统的临时目录（`$TMPDIR`，否则 `/tmp`；Windows 下为 `%TEMP%`）——不要保存在当前工作区中。

在文档中包含一个“建议使用的技能（suggested skills）”小节，列出下一个 Agent 应通过 Skill 工具调用哪些技能。

不要重复其他产物（spec、计划、ADR、issue、commit、diff）中已经记录的内容，改用路径或 URL 引用即可。

脱敏并剔除任何敏感信息，例如 API 密钥、密码或个人身份信息。

如果用户传入了参数，将其视为下一轮会话的聚焦方向，并据此定制交接文档。