
# 产物格式与 front matter 契约

读侧按这个布局写盘，写侧接受同一套单篇文件。一个格式，两个方向。

## 1. 包结构

```
<out>/
├── README.md                     # 索引表 + 拓扑树（templates/index.md + templates/topology.md）
├── issues/
│   └── NNNN-<kind>-<slug>.md     # 每单一份（templates/issue.md）
└── data/
    ├── issues.json               # gh issue list 原始载荷
    └── relations.json            # 每单的 parent / sub_issues / blocked_by / blocking / comments
```

`-Zip` 把同一棵树压缩，压缩包以输出目录名作根。

## 2. Front Matter

只支持标量与行内数组，不支持嵌套 map 与块序列。写侧就按这个子集解析。

| 字段 | 推送时必需 | 含义 |
| :--- | :--- | :--- |
| `key` | 是 | 本地身份，供 `parent` / `blocked_by` 引用。读侧写的是 issue 编号；手写计划可用任意稳定串（`S1-01`、`T-spec`）。 |
| `title` | 是 | issue 标题。 |
| `labels` | 否 | `[a, b]`。标签必须已在仓库存在，否则推送失败。 |
| `state` | 否 | `open` / `closed`；读侧记录，写侧忽略。 |
| `kind` | 否 | `epic` / `ticket` / `issue`，或 `-KindRules` 给出的名字。仅供展示。 |
| `parent` | 否 | 父单的 key，会挂成 sub-issue。 |
| `blocked_by` | 否 | `[key, key]`，连成原生 `blocked_by` 边。 |
| `blocks` | 否 | 反向视图，读侧计算。写侧忽略。 |
| `number` | 否 | 真实编号，读写两侧都会填。 |
| `url` | 否 | 规范链接。 |
| `created_at` / `updated_at` | 否 | ISO 时间戳。 |

不认识的键原样保留，写侧忽略。

## 3. kind 判定

优先级：

1. `-KindRules "spec=^Spec\s,ticket=^S\d-\d\d"` —— 标题上第一条命中的正则胜出。
2. 没有规则命中：有子单为 `epic`，有父单为 `ticket`，否则 `issue`。

kind 只影响文件名与索引列，不参与任何 API 调用。

## 4. 模板

`templates/` 下是三份纯 Markdown，占位符写作 `{{PLACEHOLDER}}`。把任意一份复制到你的包旁边、用 `-Templates <dir>` 指过去即可覆盖内置版式，不用改代码。

| 模板 | 占位符 |
| :--- | :--- |
| `issue.md` | `{{FRONT_MATTER}}` `{{TITLE}}` `{{META_TABLE}}` `{{BODY}}` `{{COMMENTS}}` |
| `index.md` | `{{REPO}}` `{{COUNT}}` `{{RANGE}}` `{{GENERATED_AT}}` `{{TABLE}}` `{{TOPOLOGY}}` `{{NOTES}}` |
| `topology.md` | `{{TREE}}` `{{NOTE}}` |

缺占位符时留空，不会让渲染失败。

## 5. 续传状态

写侧在映射文件旁写 `<mapping>.state.json`：

```json
{ "pending_creates": ["S3-02"], "pending_parents": ["S3-02"], "pending_edges": [["S3-04", "S3-01"]] }
```

`-Resume` 读它继续。`-DryRun` 永不写状态。
