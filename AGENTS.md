<!-- PENGJ_TEMPLATE_START -->

# unbound-ui 编码规范
- **构建与测试**：统一走 `just check` / `just test`。
- **回复前提交**：验证通过后先 commit & push 再回复（约定式提交：`type(scope): 中文摘要`）。
- **代码注释**：注释强制中文，专有名词保留英文。
- **代码语言守卫**：代码与字符串一律英文（中文仅限注释）；例外标 `lang-ok` / `lang-allow`，自检跑 `just gate-check`。
<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属规范可写在下方（锚点之外），模板更新时将完整保留用户内容 -->

## Agent skills

### Issue tracker

issue 与 spec 存放在本仓库的 GitHub Issues 中，使用 `gh` CLI 操作。详见 `docs/agents/issue-tracker.md`。

### Triage labels

沿用默认五角色同名标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。详见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文（single-context）布局：根目录 `GLOSSARY.md` + `docs/adr/`，缺失时静默继续。详见 `docs/agents/domain.md`。
