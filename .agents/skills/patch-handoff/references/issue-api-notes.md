
# GitHub Issues 接口备忘

本页全部在 `gh` 2.94.0 上实测过，不要凭印象替换。

## 1. 读：一条 `gh issue list --json` 就够

可用字段（`gh issue view` 是同一套）：

```
assignees author blockedBy blocking body closed closedAt closedByPullRequestsReferences
comments createdAt id isPinned issueType labels milestone number parent projectCards
projectItems reactionGroups state stateReason subIssues subIssuesSummary title updatedAt url
```

`parent`、`subIssues`、`blockedBy`、`blocking`、`subIssuesSummary` 覆盖了本技能需要的全部关系。一次分页调用胜过 N+1 次请求。

### 形状坑：是 connection 不是数组

`subIssues`、`blockedBy`、`blocking` 是 GraphQL connection 对象：

```json
{ "blockedBy": { "nodes": [ { "number": 26, "title": "...", "state": "OPEN" } ] } }
```

`parent` 要么是单个 issue 对象，要么是 `null`。列表结果里 `.blockedBy | map(.number)` 会报 `expected an object but got: array` —— 要读 `.blockedBy.nodes[]`。碰 `.number` 之前先判空 `parent`。

### 评论

`list` 上的 `--json comments` 不保证带完整正文。需要全文时逐单取 `/repos/{owner}/{repo}/issues/{number}/comments?per_page=100`。

## 2. 写：只有两个 REST 端点

`gh issue` 提供 create / list / view / close / comment / edit / develop / lock / pin / transfer / reopen / delete，**没有**子工单、**没有**依赖子命令，这两类边只能走 `gh api`：

```powershell
# 父子
gh api --method POST "repos/$Repo/issues/$ParentNumber/sub_issues" -F "sub_issue_id=$ChildDbId"

# 依赖（child 被 blocker 阻塞）
gh api --method POST "repos/$Repo/issues/$ChildNumber/dependencies/blocked_by" -F "issue_id=$BlockerDbId"
```

两个载荷要的都是**数据库 id**，不是 issue 编号：

```powershell
gh api "repos/$Repo/issues/31" --jq ".id"
```

写之前先读现状（`GET .../sub_issues`、`GET .../dependencies/blocked_by`），重跑时只补缺口。

## 3. 运行约束

- **墙钟**：托管 agent shell 里长时间上传会在 120s 附近被杀。单次运行给足预算（默认 110s），落进度、可续传，而不是从头重跑。
- **传输抖动**：`net/http: TLS handshake timeout` 与 `graphql: EOF` 会在网络正常时偶发。每次调用重试几次、短退避，再判定失败。
- **速率**：创建本身便宜，但每次「写前读」都算一次调用。同一次运行内缓存数据库 id。
- **顺序**：先把所有 issue 建完，再连边。`blocked_by` 要求两端都已存在。
