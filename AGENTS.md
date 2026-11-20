<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范
- **构建与测试**：统一走 `just check` / `just test`。
- **回复前提交**：验证通过后先 commit & push 再回复（约定式提交：`type(scope): 中文摘要`）。
- **代码注释**：注释强制中文，专有名词保留英文。
- **代码语言守卫**：代码与字符串一律英文（中文仅限注释）；例外标 `lang-ok` / `lang-allow`，自检跑 `just gate-check`。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->

## 命令排队

- 所有重型命令（编译、测试、跑分、着色器编译、.NET 构建与截图采集）必须经 `just` 配方；没有现成配方时用 `just q-run <cmd>`，跑分用 `just q-bench <cmd>`，严禁裸跑 `cargo` / `dotnet` / `glslc` 等。
- 开窗交互程序只把编译段排队，编译完直接运行二进制，避免常驻窗口长期占用全局锁或被降到 Idle 优先级。

## Agent skills

### Issue tracker

issue 与 spec 存放在本仓库的 GitHub Issues 中，使用 `gh` CLI 操作。详见 `docs/agents/issue-tracker.md`。

### Triage labels

沿用默认五角色同名标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。详见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文（single-context）布局：根目录 `GLOSSARY.md` + `docs/adr/`，缺失时静默继续。详见 `docs/agents/domain.md`。
