<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范

## 通用约定
- 编译/检查/测试/跑分统一走 `just`（`just check`、`just test`、`just q-run <cmd>`），不裸跑 `cargo`/`ninja`/`cmake`；详见 `.agents/skills/bque/SKILL.md`。
- 完成修改并通过验证后，先 commit & push 再回复；纯问答或构建/测试失败待排查时除外。
- 克隆后跑 `just doctor --fix`（没有 just 时用 `sh scripts/doctor.sh --fix` / `pwsh -NoProfile -File scripts/doctor.ps1 -Fix`）：检查工具链、激活提交钩子、创建 agent 客户端工作区链接（`.trae` / `.claude/skills` / `.workbuddy/skills`），并执行 `scripts/doctor.d/` 下的项目自检与修复。缺 `pengj-templates-cli` 是**错误**而不是警告（门禁与链接都依赖它），唯一的联网入口是 `--install-cli`，`--fix` 永不联网。
- 提交标题必须用中文（至少 2 个汉字），type/scope 用英文；任何提交入口（含 /commit 命令）都不得写纯英文标题，commit-msg 钩子会拦截。
- 提交门禁由 `.githooks/` 执行，激活入口是 `just doctor --fix`（或 `sh scripts/doctor.sh --fix`）：doctor 会调用 `.githooks/install.sh`，后者只需要 git 与一个 POSIX shell，所以激活钩子不需要 CLI。门禁本体在 `pengj-templates-cli` 里：没装它时钩子会拦下提交并打印安装命令。gate 层会自动带上 `ci` 层，服务端跑的是同一个二进制，并把它设成必需状态检查。
- 规则只有一套：钩子只负责找到 CLI 并把活交给它，模板不带兜底脚本。没装 CLI 的机器在装好之前无法在本地提交，刻意不提供一套可能与 CI 判决不一致的更宽松检查。同样的规则由 `.github/workflows/gate.yml` 的 `gate` 任务与 `.github/rulesets/gate.json` 的必需状态检查在服务端执行。
- 提交前自查标题：必须匹配 `type(scope): 摘要`，type 取自 `.gate.toml` 的 `commit.types`，摘要至少 2 个汉字，且不以句号结尾，标题不超过 100 字符。门禁会兜住全部，但先自查比被拦下更快。
- 代码语言守卫：每次提交按 `.gate.toml` 的 `mode` 校验标识符与字符串文案——英文编程拒绝非英文，中文编程拒绝非中文。中文模式只判定面向人的文案，命令行参数、路径、协议字段名、内嵌源码等机器字面量不算违规（仍计入分母，避免额度被稀释）。每个文件容忍小比例（默认 1%）字符串例外；真实例外必须在源码里认证：行尾 `lang-ok: 理由`，或文件头附近 `lang-allow: 理由`。全量自检：`just gate-check`。
- 改动涉及模块边界、公共 API、数据流、生命周期或新增模块/子系统时，同步更新对应的架构/设计文档再提交。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->

## 代码注释与文档语言规范
- **代码注释必须使用中文**：所有源码文件（`.rs`、`.wgsl`、`.js`/`.mjs`、`.sh` 等）中的文档注释（`//!`、`///`）与行内注释（`//`、`/* */`）必须使用中文编写；API 标识符、符号名、路径与专有名词（如 `SlotMap`、`Vulkan`、`BDA`、`MTSDF`、`RelayoutBoundary`）可保留英文原文。
- **架构文档规范**：`docs/architecture/` 的写法遵循该目录 `README.md` 末尾的《架构文档编写规范》（分层导航、五段式、禁代码级符号、必须配 Mermaid），统一使用中文编写。修改代码前先核对 `docs/architecture/README.md` 路由至对应领域文档。

## 代码语言（本仓库 `chinese_programming = false`）
- **标识符一律英文**：函数名、类型名、字段名、局部变量、测试函数名都只用 ASCII；中文只出现在注释里。
- **字符串文案默认英文**：测试断言、`expect` 提示、错误消息、示例数据一律英文，禁止中文标点。
- **按占比容忍，例外必须认证**：每个文件单独算占比——字符串违规数 ÷ 字符串总数 ≤ 1%、标识符违规数 ÷ 标识符总数 ≤ 0（额度和违规数都向下取整）。真实需要的例外在源码里认证：
  - 单行放行：该行注释写 `lang-ok: 理由`；
  - 整文件放行：文件头注释写 `lang-allow: 理由`，仅限“中文即数据”的文件（i18n 词条表、拼音字典、双语展厅文案、CJK 测试样本）。
- **门禁**：pre-commit 调用 `pengj-templates-cli gate hook pre-commit`，对暂存的 `.rs` 跑 rustfmt 与语言守卫；全量审计运行 `just gate-check`（等价于 `pengj-templates-cli gate check`，可加 `--list` 列出全部违规、`--json` 输出机器可读报告、`--max-ratio` 临时放宽阈值）。
- **判定方向**来自 `.pengj-templates.json` 的 `options.chinese_programming`（`false` 查非英文、`true` 查非中文），`PENGJ_CODE_LANG=english|chinese` 可临时覆盖。

## Agent skills

### Issue tracker

issue 与 spec 存放在本仓库的 GitHub Issues 中，使用 `gh` CLI 操作。详见 `docs/agents/issue-tracker.md`。

### Triage labels

沿用默认五角色同名标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。详见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文（single-context）布局：根目录 `GLOSSARY.md` + `docs/adr/`，缺失时静默继续。详见 `docs/agents/domain.md`。
