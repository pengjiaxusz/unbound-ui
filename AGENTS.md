<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范

## 通用约定
- 编译/检查/测试/跑分统一走 `just`（`just check`、`just test`、`just q-run <cmd>`），不裸跑 `cargo`/`ninja`/`cmake`；详见 `.agents/skills/bque/SKILL.md`。
- 完成修改并通过验证后，先 commit & push 再回复；纯问答或构建/测试失败待排查时除外。
- 提交标题必须用中文（至少 2 个汉字），type/scope 用英文；任何提交入口（含 /commit 命令）都不得写纯英文标题，commit-msg 钩子会拦截。
- 改动涉及模块边界、公共 API、数据流、生命周期或新增模块/子系统时，同步更新对应的架构/设计文档再提交。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->

## 代码注释与文档语言规范
- **代码注释必须使用中文**：所有源码文件（`.rs`、`.wgsl`、`.js`/`.mjs`、`.sh` 等）中的文档注释（`//!`、`///`）与行内注释（`//`、`/* */`）必须使用中文编写；API 标识符、符号名、路径与专有名词（如 `SlotMap`、`Vulkan`、`BDA`、`MTSDF`、`RelayoutBoundary`）可保留英文原文。
- **架构文档规范**：`docs/architecture/` 的写法遵循该目录 `README.md` 末尾的《架构文档编写规范》（分层导航、五段式、禁代码级符号、必须配 Mermaid），统一使用中文编写。修改代码前先核对 `docs/architecture/README.md` 路由至对应领域文档。

## 标识符与字符串语言（本仓库 `chinese_programming = false`）
- **标识符一律英文**：函数名、类型名、字段名、局部变量、测试函数名都只用 ASCII；中文只出现在注释里。
- **字符串默认英文**：测试断言文案、`expect` 提示、错误消息、示例数据一律英文，禁止中文标点。
- **中文串必须认证**，两种方式：
  - 单行放行：该行注释写 `zh-ok: 理由`；
  - 整文件放行：文件头注释写 `zh-strings-allow: 理由`，仅限“中文即数据”的文件（i18n 词条表、拼音字典、多语言示例）。
- **门禁**：pre-commit 由 `scripts/check-zh-strings.mjs` 扫描暂存的 `.rs`（标识符含中文直接拒绝，字符串含中文需持证）。全量审计直接运行该脚本，不带参数即扫描 `crates/` 与 `apps/`。
- **历史遗留**：`apps/showcase`、部分控件单测里的中文文案尚未清理，只有被暂存时才会报错；改动这些文件时把涉及行改成英文或按需加认证标记。
