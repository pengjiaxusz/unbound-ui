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
- **架构文档规范**：`docs/architecture/` 遵循 `.agents/skills/arch-align/SKILL.md` 规范，统一使用中文编写。修改代码前先核对 `docs/architecture/README.md` 路由至对应领域文档。
