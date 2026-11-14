---
name: subrepo-sync
description: >-

  通用的子仓库与子模块依赖同步、变动语义聚类分析与宿主适配技能。自动化比对子仓库未应用提交（自旧指针到目标最新提交）、按约定式提交及领域子系统（破坏性变动、动效、高亮、样式、设计令牌、规范组件等）聚类分析影响面、执行安全更新签出、指导宿主代码重构替换与门禁核验。当需要升级 Git Submodule、CMake FetchContent 或独立子仓库依赖、同步子模块最新提交、排查子仓库变动影响面或适配新特性时使用。 Triggers: subrepo-sync, submodule-sync, subrepo, submodule, update-subrepo, upgrade-subrepo, sync-submodule.

---

<!-- PENGJ_TEMPLATE_START -->

# 子仓库同步与升级 (Subrepo Sync)

用于在宿主项目中受控升级外部依赖子仓库（Git Submodule、CMake FetchContent、独立子仓库等），提取未应用提交、按子系统聚类分析影响面、指导业务组件重构与门禁回归。

## 快速开始

在宿主仓库根目录下执行内置脚本，自动化分析提交差异与影响面：

```powershell
# 1. 查看未应用提交记录并聚类分析影响面
pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "<子仓库路径>"

# 2. 确认升级，附加 -Update 一键签出最新目标提交
pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "<子仓库路径>" -Update
```

## 标准升级工作流

```
[1. 识别基线与目标] ──► [2. 提取未应用提交与聚类] ──► [3. 安全签出与更新] ──► [4. 宿主适配与重构] ──► [5. 门禁验证与提交]
```

### 步骤 1：识别基线与目标
1. 读取当前子仓库锁定指针：`git -C <子仓库路径> rev-parse HEAD`。
2. 探测目标更新源：
   - 本地同级联调目录（`..\<subrepo>`）；
   - 环境变量指定目录（`$env:<NAME>_DIR`）；
   - 远程主干分支（`origin/main` 或 `origin/master`）。
3. **多个待合入提交各自 bump 过指针时，先判先后再定目标**：新指针必须是旧指针的**后代**才安全，
   否则合入的功能会**静默失效**（QML/接口层面表现为属性或符号不存在）。
   取各提交里的指针值（`git show <sha>:<锁文件> | grep <PIN 变量>`），在子仓库内
   `git fetch origin <sha>` 之后（本地常缺该对象，**必须先 fetch**）用
   `git merge-base --is-ancestor <旧> <新>` 判定；最后再对照实际构建用的源码目录确认目标能力确实存在。

### 步骤 2：提取未应用提交并进行语义聚类
提取区间提交（`git log <旧Commit>..<新Commit> --oneline --no-merges`），按子系统聚类：
- **💥 破坏性变动 (Breaking Changes)**：`feat!:`, `fix!:`, `BREAKING CHANGE:` ➔ 重点研判 API 破坏与类型变动；
- **🎬 动效与过渡 (Motion)**：`motion`, `easing`, `animate` ➔ 检查退出动画、缓动曲线与时钟坍缩契约；
- **🎨 设计令牌与色彩 (Tokens & Themes)**：`token`, `theme`, `palette` ➔ 遵循背景/前景色成对自洽律；
- **📐 样式与层叠 (Styles & Layers)**：`styles`, `cascade-layer`, `css` ➔ 确保样式层叠优先级不被覆盖；
- **🧩 规范组件与接口 (Components)**：`component`, `feat(...)` ➔ 遵循组件自举律，替换业务自研轮子；
- **⚡ 性能与 🐛 缺陷修复 (Perf & Fixes)**：`perf:`, `fix:` ➔ 清理宿主内临时规避代码。

### 步骤 3：安全签出与指针更新
- 确认子仓库与宿主工作区干净无脏改动（`git status --short`）；
- 签出目标提交（`git checkout <新Commit>` 或执行项目专用更新命令）。

### 步骤 4：宿主代码适配与重构
1. 检索宿主项目中是否有对应功能的自研手写实现，将其重构替换为规范组件；
2. 严格遵循宿主架构红线（见托管块外的项目专属区）。

### ⚠️ 升级后先重建测试目标（陈旧产物陷阱）

常规构建命令通常**只产出主程序，不产出测试可执行文件**；而依赖模块（静态库、资源、QML 等）
是编译进各个测试可执行文件的。升级依赖指针后若直接跑测试门禁，**旧可执行文件会用旧依赖模块去加载
新源码树里的资源**，报出一串看似自伤的错：

- `Cannot assign to non-existent property "<新增属性>"`；
- `Type <宿主组件> unavailable`；
- 若同时并行改过宿主自身的代码，还会冒出毫不相关的同类报错（同因：可执行文件早于该属性入库）。

**这些是陈旧产物的假阳性，不是回归**——先重建（重链）测试目标即可消除。
动手排查前，先确认测试可执行文件的构建时间是否晚于本次依赖更新。

### 步骤 5：门禁验证与规范提交
1. 执行宿主技术栈的编译、类型检查与单元测试；
2. 运行项目专属的卫生门禁与界面真实环境核验；
3. 撰写规范提交信息（如 `chore(deps): 更新 <subrepo> 至 <hash> 并适配组件规范`），提交后立即 push。

## 详细指南与实战案例（渐进式披露）

- [子仓库系统架构与依赖模式详解](REFERENCE.md)
- [历史真实升级与组件重构案例](EXAMPLES.md)

<!-- PENGJ_TEMPLATE_END -->


<!-- 以下为项目专属区域：模板更新只替换上方托管块，本区域归项目所有、完整保留。 -->
## 项目专属子仓库配置与门禁契约

> 在此处声明本项目的子仓库路径、领域专属聚类关键词、宿主验证门禁与架构红线。

### 1. 托管子仓库登记
| 子仓库名称 | 相对路径 | 依赖模式 (Submodule / FetchContent / 本地) | 上游跟踪分支 |
| :--- | :--- | :--- | :--- |
| `example-subrepo` | `submodules/example-subrepo` | Git Submodule | `origin/main` |

### 2. 宿主架构红线
- **单真相源驱动**：严禁在依赖缓存目录中直接修改或分叉；所有通用组件与契约必须源自 upstream。
- **组件自举律**：上游一旦推出规范控件，必须及时重构并移除宿主自研的重复手写实现。

### 3. 项目专属验证门禁
```powershell
# 静态与编译检查
# cargo check --workspace
# pnpm build

# 自动化测试与卫生门禁
# cargo test --workspace
# pwsh tools/hygiene.ps1
```
