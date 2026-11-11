---
name: template-sync
description: >-

  基于 pengj-templates 的分层模板同步、项目纳管与模板漂移巡检技能。针对存量或新项目纳管（adopt），自动诊断技术栈、编程范式与提交习惯并提供推荐方案咨询用户；巡检查验下游受管脚本篡改与托管块漂移（audit）；同步上游模板最新改动到已有项目（update）；裁决更新差异与接管过渡区（needs_review / conflicted / 纳管过渡区）或验证下游项目对齐。
  Triggers: template-sync, 模板同步, 更新模板, 应用模板, 纳管项目, 同步模板, 模板巡检, 模板漂移, 检查模板改动, update-template, apply-template, adopt-project, 模板更新, template-audit, template-diff.

---

<!-- PENGJ_TEMPLATE_START -->

# 模板同步与项目纳管 (Template Sync)

用于将 `pengj-templates` 上游模板更新同步到下游项目，或将分层模板应用/纳管到存量现有项目的工作流。

## 快速开始

### 极速流水线通道（推荐）

优先调用技能内置的极速流水线脚本 `sync-template.ps1`。该脚本毫秒级直调预编译二进制产物、支持批量项目目录、原生守护「精准暂存红线（严禁 `git add .`）」、并可在单次工具调用内闭环「扫描 -> 更新 -> 精准暂存 -> 约定提交 -> 推送」：

```powershell
# 快速预检：1 秒内快速诊断单项目或多项目的漂移状态（Dry Run）
pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("<项目目录1>", "<项目目录2>")

# 一键流水线：批量更新、精准暂存、生成约定式提交并推送到远端（一次调用全自动闭环）
pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("<项目目录1>", "<项目目录2>") -Apply -Commit -Push
```

### 原生 CLI 命令

在 `pengj-templates` 仓库根目录运行，或确保 `pengj-templates-cli` 在 PATH 中 / 已设置 `PENGJ_TEMPLATES` 环境变量：

```powershell
# 0. 巡检项目模板对齐状态（支持单目录 --dir 或多目录 --dirs，只读排查篡改与漂移）
cargo run -p pengj-templates-cli -- audit --dirs <目录1,目录2> --diff

# 1. 更新已纳管项目（建议带上 --sync-skills 确保纯技能资产平滑对齐覆盖）
cargo run -p pengj-templates-cli -- update --dirs <目录1,目录2> --sync-skills

# 2. 纳管/应用模板到存量项目（尚无 manifest）
cargo run -p pengj-templates-cli -- adopt --dir <目标项目目录> --layers <层1,层2> [选项]
```

## 工作流

### 1. 状态诊断与模式判定

检查目标项目根目录是否存在 `.pengj-templates.json`：

- **巡检排查**：需检查下游是否出现私自修改脚本或违规漂移，进入 **Audit 模式（步骤 2C）**。
- **存在 manifest**：说明项目已纳管，直接进入 **Update 模式（步骤 2A）**。
- **不存在 manifest**：说明是存量或未受管项目，进入 **Adopt 模式（步骤 2B）**。

### 2A. Update 模式（同步上游更新）

1. 执行极速脚本或更新命令：
   ```powershell
   # 推荐：极速流水线脚本
   pwsh .agents/skills/template-sync/scripts/sync-template.ps1 -Projects @("<目标项目目录>") -Apply

   # 或通过 CLI 直接调用
   cargo run -p pengj-templates-cli -- update --dir <目标项目目录> --sync-skills
   ```
2. 引擎自动从 manifest 读取固化的选项（`edition`、`skills`、`skill_lang` 等）并重渲染比对：
   - **纯受管技能资产（`.agents/skills/`）**：开启 `--sync-skills` 时（极速脚本默认启用），纯工具技能与脚本即使无托管块也会平滑对齐覆盖为最新模板版本，不判定为冲突；
   - **磁盘未改动文件**：直接覆盖为最新模板；
   - **含受管块文本（`PENGJ_TEMPLATE_START/END`）**：受管块内原位替换，块外项目专属内容完整保留；
   - **TOML / JSON（`.cargo/config.toml`、`package.json`）**：结构化并集合并；
   - **无受管块且用户已修改的文件**：标记为 `conflicted` 并跳过（绝不静默覆盖用户改动）。

### 2B. Adopt 模式（存量/新项目纳管与推荐）

面对未受管的存量或新项目，**严禁盲目直接执行 adopt**。AI 必须先自动进行项目深度诊断、生成首选推荐方案，并主动咨询用户进行确认与选择：

#### 1. 自动化技术栈与特征诊断（推导推荐方案）

AI 应首先巡检目标项目的工作区，按以下维度完成自动化诊断：
- **技术栈与适用层判定**：
  - 检查项目根目录与源码：包含 `Cargo.toml` 则命中 Rust 技术栈，推荐 `rust` 或 `rust-workspace` 层；**非 Rust 项目严禁引入 Rust 相关层**。
  - 检查工具链：是否包含 Node.js / 前端生态（`package.json`）？若有或用户希望本地 Git Hooks 物理拦截提交，可推荐 `lefthook`；若为纯脚本/无 Node 生态轻量项目，默认不选 `lefthook`（避免引入 node 依赖），作为可选附加项提示。
  - 编辑器配置：推荐引入 `vscode` 层，提供文件折叠嵌套规则（`explorer.fileNesting`），现有 `.vscode` 配置会自动并集合并。
  - `common` 与 `agent` 为所有项目的基础与核心层（必选）。
- **编程范式检测（`--chinese` 中文编程）**：
  - 检查是否存在中文目录（如 `源码/`、`配置/`、`动作/`）或代码中大量使用中文类名/函数/变量标识符。
  - 命中时必须**强烈推荐开启 `--chinese`**，为项目注入中文化命名与中英拼接规范。
- **提交习惯检测（`--commit-zh` / `--commit-and-push`）**：
  - 运行 `git log -n 5 --oneline` 检查历史提交信息。
  - 提交使用中文标题与正文时，保持 `--commit-zh`（默认开启）。
  - **强烈推荐开启 `--commit-and-push`**，落实「回复前提交门禁（硬性拦截，禁止堆积）」原则，确保 AI 助手任务完工后自动提交并推送，杜绝改动堆积。
- **存量规范与资产评估**：
  - 若已有 `AGENTS.md` 或 `GEMINI.md`，引擎将以置顶插入（Prepend）方式注入托管块，现有专属规范完整保留在块外，绝不覆盖。
  - 若已有自定义技能（无托管块），引擎会自动接管并将原内容下移至纳管过渡区（`needs_review`）。
- **技能集精选（`--skills`）**：
  - 核心基线：`commit`（约定式提交）、`template-sync`（模板同步更新）。
  - 架构与协作：`arch-align`（架构对齐）、`git-workflow`（通用 Git 操作、约定式提交与线性分支同步）。
  - 辅助增强：`caveman`（token 压缩）、`grill-me`（方案质询）、`subrepo-sync`（子模块/子仓库同步）、`write-a-skill`（技能创建）。默认全选或按需勾选。

#### 2. 向用户呈现诊断报告并咨询选择（必须给推荐方案）

必须以结构化格式向用户呈现诊断结论并提供选择方案：
- **项目特征摘要**：技术栈、中文编程范式、提交规范、现有规范保护说明；
- **首选推荐方案（标记为 `(Recommended)`）**：列出最适合该项目的层组合（如 `--layers common,agent,vscode`）、选项参数（如 `--chinese`、`--commit-and-push`）及选型理由；
- **备选/增选方案**：列出其他可行组合（如追加 `lefthook` 强化本地 Git 拦截，并说明需引入 Node 依赖的利弊）；
- **完整 CLI 命令预览**：给出对应的执行命令示例；
- **咨询与确认**：通过交互式选择（如 `ask_question` 或结构化提问）征询用户意见，获得用户确认或自定义调整。

#### 3. 执行纳管命令

在用户确认后，执行纳管命令：
```powershell
cargo run -p pengj-templates-cli -- adopt --dir <目标项目目录> --layers <层1,层2> [选项]
```
*(注意：`--dir` 用于指定目标项目目录，请勿使用裸路径位置参数)*

### 2C. Audit 模式（模板巡检与漂移排查治理）

用于检测下游项目是否存在私自篡改模板脚本或越界修改受管块的只读排查工作流：
```powershell
cargo run -p pengj-templates-cli -- audit --dir <目标项目目录> --diff
```

**漂移排查与治理决策树**：
- **`[项目专属定制]`（合规）**：改动完全处于 `PENGJ_TEMPLATE_START/END` 托管块之外的项目专属区（如 `SKILL.md` 门禁与声明、`AGENTS.md` 领域规范），属于标准合规定制。
- **`[上游有更新]`（待同步）**：上游模板已演进，下游未做修改，可直接运行 `update` 安全同步。
- **`[托管块被篡改]` / `[受管文件被改]`（违规严重漂移）**：
  - **判定 1（具通用价值）**：若下游改动解决了某种通用痛点（如分支自适应推导、常见工具链兼容），**必须反馈至上游 `pengj-templates` 进行通用化重构**，在上游发布后一键 `update` 抹平下游 hack。
  - **判定 2（项目特殊环境）**：若为下游独有的特殊构建目录、缓存或环境，**严禁直接修改脚本或受管块**，必须利用模板提供的声明机制（如 `SKILL.md` 的「忽略未追踪路径正则」或项目专属区）进行声明配置，并把受管文件还原为纯净模板版本。
  - **判定 3（严禁私留 Hack）**：下游项目严禁留存对模板托管脚本及托管块内的私自修改，必须时刻保持 audit 违规项为零。

### 3. 结果解析与后处理（关键步骤）

检查 CLI 输出报告中的各项分类：

- **`needs_review`（过渡区合并与审核）**：
  - 若存量自定义技能被模板整页接管，原正文会移至 `<!-- === PENGJ_ADOPT_TRANSITION_ZONE === -->`（纳管过渡区）。
  - 协助用户将过渡区中独有的领域检查项合并入托管块外的「项目专属区」。
  - 合并完成后，**删除过渡区注释及内容**。
- **`conflicted`（无受管块冲突）**：
  - 查看冲突路径与原因，按需手动将模板变更合入对应文件。
- **配置与工具链接线核验**：
  - 检查 `commitlint.base.js` 自动接线、`.vscode/settings.json`、`.cargo/config.toml` 合并是否符合预期。

### 4. 验证与按需精准提交

1. 进入目标项目目录，执行对应技术栈的验证检查：
   ```powershell
   # 如 Rust 项目
   cargo check; cargo test
   # 如前端项目
   pnpm build
   ```
2. **提交策略与精准提交流程**：
   - **默认不提交**：执行 `update` 或 `adopt` 后，默认仅在工作区完成更新与核验，**不要自动提交**，等待用户确认或显式指令。
   - **用户要求提交时的精准提交（红线）**：当用户明确要求提交时，**只能单独 `git add <file>` 本次模板同步/纳管所修改或新增的文件**（严格对照 update 报告清单），**严禁 `git add .` 或 `git commit -a`**，防止误将工作区既有的无关改动或脏文件一并带入提交。
   - 提交信息采用约定式提交（如 `chore(templates): 同步上游模板更新` 或 `chore(templates): 纳管项目分层模板`），并在提交后推送。

<!-- PENGJ_TEMPLATE_END -->


<!-- 以下为项目专属区域：模板更新只替换上方托管块，本区域归项目所有、完整保留。 -->
## 项目专属同步配置与快捷方式

> 可在此登记本项目常用的上游模板地址、默认层组合、前后置同步脚本等信息。
