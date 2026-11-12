<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范

> 供 AI 编码助手在修改本仓库时遵循。由 pengj-templates 的 `agent` 层按所选层与选项生成。

## 通用约定
- **统一任务入口与防卡顿（严禁裸跑编译器）**：所有编译、检查、单测与跑分必须统一通过 `just`（如 `just check`、`just test`、`just bench`）或 `just q-run <cmd>` 调度，严禁直接在命令行裸跑 `cargo`/`ninja`/`cmake` 等底层构建工具，确保全局跨工作树排队调度与 CPU 降级防卡顿（详见 `.agents/skills/bque/SKILL.md`）。
- **提交并推送（硬性）**：每完成一轮修改并通过验证（或任务收尾）后，先 commit & push 再结束回复；纯问答、或构建/测试失败需报告时除外。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->


