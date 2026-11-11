
# 子仓库系统架构与依赖模式详解 (REFERENCE.md)

本文档深入解析常见子仓库管理模式、双层语义聚类引擎工作原理，以及更新过程中的异常处置与回滚策略。

---

## 一、常见子仓库依赖模式对比

在多仓库协同架构中，宿主通常通过以下三种模式引入子仓库：

| 依赖模式 | 适用场景 | 版本锁定机制 | 本地联调机制 |
| :--- | :--- | :--- | :--- |
| **1. Git Submodule（标准子模块）** | 生产标准、跨端组件库、共享算法模块 | 由宿主仓库精确记录 Commit Hash（指针） | 通过 `git checkout` 或环境变量重定向到同级仓库 |
| **2. CMake FetchContent / 缓存拉取** | C/C++ 依赖包、构建期自动拉取 | 由 CMake 脚本或锁定提交参数控制 | 通过 `-D<NAME>_DEV_LOCAL=ON` 重定向到本地目录 |
| **3. 本地同级 Worktree 联调** | 日常双向联调、特性快速迭代 | 软链接、构建别名（Vite alias / Cargo path） | 实时热重载（HMR），免打包直接生效 |

### 模式切换与别名重定向最佳实践
在前端工程（如 Vite / Webpack）或 Rust/CMake 工程中，建议通过环境变量分层挂载路径别名：
- 默认读取 `submodules/<name>` 源码；
- 当检测到 `$env:<NAME>_DIR` 或存在同级开发目录 `..\<name>` 时，动态重定向别名，避免手动修改源码路径；
- 避免硬编码绝对路径到提交代码中。

---

## 二、双层语义聚类引擎原理

更新子仓库时，面对几十甚至数百条提交历史，人工逐行研判极为耗时。`show-unapplied-commits.ps1` 采用双层分类过滤：

### 1. 第一层：Conventional Commits 语义分类
- **💥 破坏性变动 (Breaking Changes)**：匹配 `^[a-f0-9]+\s+[a-z0-9\-]+!:` 或 `BREAKING CHANGE:`。
  - 此类变动意味着 API 签名调整、破坏性字段重命名或行为不兼容，必须由 Agent/开发者优先审查并适配宿主调用处。
- **✨ Features (`feat:`)**：新功能、新组件、新导出能力。
- **🐛 Bug Fixes (`fix:`)**：缺陷修复、边界保护。
- **⚡ Performance (`perf:`)**：性能优化。
- **♻️ Refactor (`refactor:`)**：内部重构，通常不破坏公开 API。

### 2. 第二层：领域子系统关键词分类
通过正则表达式对提交信息的主题与正文进行关键词匹配：
- **动效与时序 (Motion)**：`motion|easing|animate|transition`
- **设计令牌 (Tokens)**：`token|theme|palette|color`
- **样式与层叠 (Styles)**：`styles|cascade-layer|layer|subpath|css`
- **规范控件 (Components)**：`component|widget|control`

### 自定义分类规则扩展（-ConfigPath）
若需为项目专属的子系统定制聚类规则，可编写 JSON 文件并通过 `-ConfigPath` 传入：
```json
{
  "categories": [
    {
      "name": "着色器与渲染管线 (Shaders & Pipeline)",
      "color": "Magenta",
      "regex": "(shader|hlsl|glsl|pipeline|render-pass)"
    },
    {
      "name": "网络与 RPC 协议 (Network & RPC)",
      "color": "Cyan",
      "regex": "(proto|grpc|rpc|packet|socket)"
    }
  ]
}
```

---

## 三、安全更新、异常处理与回滚策略

### 1. 脏工作区防御
在执行 `-Update` 时，脚本会先检查子仓库的 `git status --short`：
- 若子仓库存在未提交修改，脚本会直接终止签出，防止覆盖本地工作；
- 开发者必须先在子仓库中执行 `git stash` 或提交，方可继续更新。

### 2. 分离头指针 (Detached HEAD) 说明
Git Submodule 更新默认会将子仓库检出到游离状态（Detached HEAD）。这是 Git Submodule 的预期行为，因为宿主仓库只记录特定 Commit Hash。
- 若需在子仓库中临时开发提交：应在子仓库中创建新分支（`git checkout -b feat/my-change`），推送到上游后，再将宿主指针指向新提交。

### 3. 指针回滚应急流程
若升级后编译失败或门禁拦截，需要快速恢复到升级前状态：
```powershell
# 1. 放弃宿主项目中对子模块指针的暂存
git restore --staged submodules/<name>

# 2. 将子仓库指针还原至宿主当前记录的提交
git submodule update --init --recursive submodules/<name>
```
