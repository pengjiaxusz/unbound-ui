# 领域文档（Domain Docs）

工程类技能在探索代码库时应如何消费本仓库的领域文档。

## 探索前先读这些

- 仓库根目录的 **`GLOSSARY.md`**，或
- 若仓库根目录存在 **`GLOSSARY-MAP.md`**：它指向每个上下文各自的 `GLOSSARY.md`，按当前话题读取相关的每一份。
- **`docs/adr/`**：读取触及你即将改动区域的 ADR。在多上下文仓库中，也要检查 `src/<context>/docs/adr/` 下的上下文专属决策。

如果以上文件都不存在，**静默继续**。不要标记它们的缺失，也不要提前建议创建它们。当术语或决策真正落定时，`/domain-modeling` 技能（通过 `/grill-with-docs` 与 `/improve-codebase-architecture` 触发）会懒创建它们。

## 文件结构

单上下文仓库（绝大多数仓库）：

```
/
├── GLOSSARY.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

多上下文仓库（根目录存在 `GLOSSARY-MAP.md`）：

```
/
├── GLOSSARY-MAP.md
├── docs/adr/                          ← 系统级全局决策
└── src/
    ├── ordering/
    │   ├── GLOSSARY.md
    │   └── docs/adr/                  ← 上下文专属决策
    └── billing/
        ├── GLOSSARY.md
        └── docs/adr/
```

## 使用术语表中的词汇

当你的输出命名一个领域概念时（在 issue 标题、重构提案、假设、测试名称中），使用 `GLOSSARY.md` 中定义的术语。不要漂移成术语表明确列为避免使用的同义词。

如果你需要的概念还不在术语表里，这是一个信号：要么你在发明项目并不使用的语言（重新审视），要么这里确实存在缺口（记下来交给 `/domain-modeling`）。

## 标出 ADR 冲突

如果你的输出与现有 ADR 矛盾，明确把它摆出来，而不是静默覆盖：

> _与 ADR-0007（事件溯源订单）冲突，但值得重开讨论，因为……_