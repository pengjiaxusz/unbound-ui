# 编写 Agent 简报（Agent Briefs）

Agent 简报是在 GitHub issue 或 PR 流转到 `ready-for-agent` 时发表的一条结构化评论。它是供离线（AFK）Agent 依此施工的权威规格说明。原始正文与讨论只是背景上下文；Agent 简报才是契约。

简报说明**Agent 应该做什么**，同时适用于两种入口：对 issue 而言，是从零构建改动；对 PR 而言，是对*现有 diff* 还有什么收尾工作要做——补全缺口、解决评审意见。两者原则完全一致，下文 PR 示例展示了写法差异。

## 原则

### 持久性优于过度精确

工单可能在 `ready-for-agent` 状态停放几天甚至几周，期间代码库会不断变化。写简报时要保证即使文件被重命名、移动或重构，它依然有效：

- **要**描述接口、类型与行为契约
- **要**点出 Agent 应查找或修改的具体类型名、函数签名或配置形状
- **不要**引用具体文件路径：路径会过时
- **不要**引用行号
- **不要**假设当前内部实现结构一成不变

### 描述行为，而非过程

描述系统**应该做什么**，而不是**如何去实现**。Agent 会亲自重新探索代码库并做出自己的实现决策。

- **好：** “`SkillConfig` 类型应接受一个类型为 `CronExpression` 的可选 `schedule` 字段”
- **差：** “打开 `src/types/skill.ts` 并在第 42 行添加 `schedule` 字段”
- **好：** “当用户不带参数运行 `/triage` 时，应看到需要关注的 issue 摘要”
- **差：** “在主处理函数里加一个 switch 语句”

### 完整的验收标准

Agent 需要知道何时才算完成。每份 Agent 简报都必须配有具体、可测试的验收标准，且每条标准都应可独立验证。

- **好：** “运行 `gh issue list --label needs-triage` 会返回已完成初步分类的 issue”
- **差：** “分诊功能应能正常工作”

### 明确的范围边界

写清哪些不在范围之内，防止 Agent 过度镀金或对相邻特性做多余假设。

## 模板

```markdown
## Agent Brief

**Category:** bug / enhancement
**Summary:** 一句话概括需要发生什么变化

**Current behavior:**
描述当前表现。对于 bug，这是出错的行为；
对于 enhancement，这是新特性所基于的现状。

**Desired behavior:**
描述 Agent 完工后应当呈现的表现。
写清边界情况与错误条件。

**Key interfaces:**
- `TypeName`：需要改什么以及为什么
- `functionName()` 返回类型：当前返回什么 vs 应该返回什么
- 配置形状：需要新增的任何配置项

**Acceptance criteria:**
- [ ] 具体、可测的验收标准 1
- [ ] 具体、可测的验收标准 2
- [ ] 具体、可测的验收标准 3

**Out of scope:**
- 本工单**不应**修改或触及的内容
- 看似相关但属于独立范畴的相邻特性
```

## 示例

### 好的 Agent 简报（Bug）

```markdown
## Agent Brief

**Category:** bug
**Summary:** 技能描述截断会在单词中间切断，导致输出残缺

**Current behavior:**
当技能描述超过 1024 个字符时，会在第 1024 个字符处硬截断，不考虑单词边界，导致结尾出现半截单词（如 "Use when the user wants to confi"）。

**Desired behavior:**
截断应在 1024 个字符之前的最后一个单词边界处断开，并追加 "..." 表示已截断。

**Key interfaces:**
- `SkillMetadata` 类型的 `description` 字段：无需改类型，但填充它的校验/处理逻辑需要尊重单词边界
- 任何读取 `SKILL.md` frontmatter 并提取描述的函数

**Acceptance criteria:**
- [ ] 不足 1024 字符的描述保持原样
- [ ] 超过 1024 字符的描述在 1024 字符前的最后一个单词边界处截断
- [ ] 截断后的描述以 "..." 结尾
- [ ] 包含 "..." 在内的总长度不超过 1024 字符

**Out of scope:**
- 修改 1024 字符上限本身
- 多行描述支持
```

### 好的 Agent 简报（Enhancement）

```markdown
## Agent Brief

**Category:** enhancement
**Summary:** 新增 `.out-of-scope/` 目录支持，用于记录被拒绝的特性请求

**Current behavior:**
当特性请求被拒绝时，issue 会被贴上 `wontfix` 标签并附一条评论关闭，但没有持久化记录该决策或理由。日后遇到类似请求时，维护者不得不凭记忆回忆或翻找历史讨论。

**Desired behavior:**
被拒绝的特性请求应记录在 `.out-of-scope/<concept>.md` 文件中，记录决策、理由以及所有请求该特性的 issue 链接。在分诊新 issue 时应检查这些文件是否命中。

**Key interfaces:**
- `.out-of-scope/` 下的 Markdown 格式：每个文件包含 `# Concept Name` 标题、`**Decision:**` 行、`**Reason:**` 行，以及带 issue 链接的 `**Prior requests:**` 列表
- 分诊工作流应在早期读取所有 `.out-of-scope/*.md` 文件，并按概念相似度匹配新进 issue

**Acceptance criteria:**
- [ ] 将特性请求以 wontfix 关闭时会在 `.out-of-scope/` 创建/更新文件
- [ ] 文件包含决策、理由以及指向已关闭 issue 的链接
- [ ] 若已存在匹配的 `.out-of-scope/` 文件，新 issue 应追加到其 "Prior requests" 列表，而不是新建重复文件
- [ ] 分诊过程中当新 issue 匹配过往拒绝记录时，会检查并向维护者提示

**Out of scope:**
- 自动匹配（由人确认是否匹配）
- 重新打开之前被拒绝的特性
- Bug 报告（只有 enhancement 拒绝记录进入 `.out-of-scope/`）
```

### 好的 Agent 简报（PR）

对于 PR，“Current behavior”描述当前 diff 的状态，简报要求 Agent 补全或修复它，而不是从零重写。

```markdown
## Agent Brief

**Category:** enhancement
**Summary:** 补全贡献者为 `triage list` 新增的 `--json` 输出参数

**Current behavior:**
该 PR 新增了将 issue 列表序列化为 JSON 的 `--json` 参数，主路径可正常工作且 diff 符合项目命令结构。还剩两处缺口：错误仍以人类可读文本打印（而非 JSON），且新参数缺少测试覆盖。

**Desired behavior:**
传入 `--json` 时，所有输出（含报错）均为 stdout 上的合法 JSON，命令退出码保持不变。不传该参数时原有文本输出保持不变。

**Key interfaces:**
- 命令的错误处理路径在 `--json` 下应输出 `{ "error": string }` 而非纯文本错误
- 复用 PR 已新增的序列化器，不要另起一套

**Acceptance criteria:**
- [ ] `triage list --json` 在成功与失败场景均输出合法 JSON
- [ ] 退出码与非 JSON 模式一致
- [ ] 补充覆盖 `--json` 成功输出与至少一个错误场景的测试
- [ ] 默认（非 JSON）输出逐字节保持不变

**Out of scope:**
- 为其他命令添加 `--json`
- 修改 PR 已定义的成功 JSON 负载结构
```

### 差的 Agent 简报

```markdown
## Agent Brief

**Summary:** 修复分诊 bug

**What to do:**
分诊那块坏掉了。看一下主文件并修好它。
大概第 150 行的函数有问题。

**Files to change:**
- src/triage/handler.ts (line 150)
- src/types.ts (line 42)
```

这份简报之所以差，是因为：
- 缺少分类（Category）
- 描述含糊（“分诊那块坏掉了”）
- 引用了很快会过时的文件路径和行号
- 没有验收标准
- 没有范围边界
- 没有说明当前行为与期望行为