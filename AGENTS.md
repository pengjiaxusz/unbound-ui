<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范

> 供 AI 编码助手在修改本仓库时遵循。由 pengj-templates 的 `agent` 层按所选层与选项生成。

## 通用约定
- **统一任务入口与防卡顿（严禁裸跑编译器）**：所有编译、检查、单测与跑分必须统一通过 `just`（如 `just check`、`just test`、`just bench`）或 `just q-run <cmd>` 调度，严禁直接在命令行裸跑 `cargo`/`ninja`/`cmake` 等底层构建工具，确保全局跨工作树排队调度与 CPU 降级防卡顿（详见 `.agents/skills/bque/SKILL.md`）。
- **模板受管区域防篡改**：带 `PENGJ_TEMPLATE_START/END` 的托管块与 `.agents/skills/*/scripts/` 脚本由 `pengj-templates` 托管，严禁私自篡改。项目专属配置请写在托管块外的项目专属区；随时运行 `cargo run -p pengj-templates-cli -- audit --dir . --diff` 巡检对齐状态。

## Rust 编码规范

- 格式化：`cargo fmt`（本地提交通过 lefthook 自动格式化暂存代码并重新暂存；CI 校验走 `cargo fmt --check`）。
- 静态检查：`cargo clippy --all-targets --all-features -- -D warnings`（`just check`），保持 warnings 清零。
- 命名：变量/函数/模块 `snake_case`，类型/特征/枚举 `CamelCase`，常量 `SCREAMING_SNAKE_CASE`。
- 《`Cargo.toml`、`.cargo/config.toml`、`rust-toolchain.toml` 由模板托管：改编译选项、工具链需同步模板（`pengj-templates-cli update`）。`src/main.rs` 归用户所有，模板更新不覆盖。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->


