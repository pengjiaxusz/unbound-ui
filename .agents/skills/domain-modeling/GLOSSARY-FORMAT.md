# GLOSSARY.md 格式规范

## 结构

```md
# {上下文名称}

{用一两句话说明这个上下文是什么、为什么存在。}

## 语言（Language）

**Order（订单）**：
{用一两句话描述该术语}
_避免使用_：Purchase, transaction

**Invoice（发票/账单）**：
交付后发给客户的付款请求。
_Avoid / 避免使用_：Bill, payment request

**Customer（客户）**：
下单的个人或组织。
_Avoid / 避免使用_：Client, buyer, account
```

## 规则

- **保持明确立场。** 同一概念存在多个词时，选出最好的一个，其余列在 `_Avoid_` 下。
- **定义要紧凑。** 最多一两句话。定义它*是什么*，而不是它做什么。
- **只收录属于本项目上下文的专属术语。** 通用编程概念（timeout、error 类型、工具模式）即使项目大量使用也不应收录。收录前先问：这是本上下文独有的概念，还是通用编程概念？只有前者属于这里。
- **按自然聚类分组。** 当术语自然聚集时用子标题分组；如果所有术语都属于同一个内聚领域，平铺列表即可。

## 单上下文 vs 多上下文仓库

**单上下文（绝大多数仓库）：** 在仓库根目录放一个 `GLOSSARY.md`。

**多上下文：** 在仓库根目录放一个 `GLOSSARY-MAP.md`，列出各个上下文、所在位置及相互关系：

```md
# Glossary Map

## Contexts

- [Ordering](./src/ordering/GLOSSARY.md)：接收并跟踪客户订单
- [Billing](./src/billing/GLOSSARY.md)：开具账单并处理收款
- [Fulfillment](./src/fulfillment/GLOSSARY.md)：管理仓库拣货与发货

## Relationships

- **Ordering → Fulfillment**：Ordering 发出 `OrderPlaced` 事件；Fulfillment 消费后开始拣货
- **Fulfillment → Billing**：Fulfillment 发出 `ShipmentDispatched` 事件；Billing 消费后生成账单
- **Ordering ↔ Billing**：共享 `CustomerId` 与 `Money` 类型
```

本技能会自动推断适用结构：

- 若存在 `GLOSSARY-MAP.md`，读取它定位各上下文
- 若仅有根目录 `GLOSSARY.md`，按单上下文处理
- 若两者都不存在，在第一个术语落定时懒创建根目录 `GLOSSARY.md`

存在多个上下文时，推断当前话题属于哪一个；若不明确，直接询问。