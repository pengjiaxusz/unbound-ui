
# Commit 提交流程与规范

约定式提交流程、回复前硬性门禁、语义拆分与立即推送安全规范。

---

## 1. 触发时机（主动触发规则）

- **代码修改与阶段性完工（回复前硬性门禁，禁止堆积）**：
  凡本轮对话存在已通过构建/测试验证的代码修改，在向用户输出最终汇报前，**必须将运行提交流程（执行检查、拆分提交并 `git push`）作为最后一个工具调用步骤**。严禁在工作区留存已验证修改直接结束回复；在 `/plan` 等规划模式下生成 `walkthrough.md` 后，亦必须紧随其后执行提交与推送，不可中途停顿。
- **任务收尾（硬性触发）**：
  任何开发或维护任务结束、向用户做最终交付前，必须确保工作区干净且已推送到远端，不留未提交修改。
- **用户显式指令**：
  用户发送 `/commit`、"提交"、"commit & push" 等。

---

## 2. 收集提交上下文（跨平台标准命令）

在仓库根目录执行：

```bash
git status
git --no-pager diff --stat
git --no-pager diff --cached --stat
```

---

## 3. 提交前完整性检查

本步骤归**项目**所有，模板不硬编码具体检查：
按主入口 `SKILL.md` 托管块外「项目专属提交流程与红线」的定义执行——跑什么命令、查哪些项，由项目自行声明。项目专属区未定义时，执行兜底自检：
- 扫 diff：改动构建/依赖、文档/AGENTS.md/配置、公开命名时对应校验构建、更新文档、跑格式化；
- **架构文档一致性检查**：触及架构（新增/调整模块边界、数据流、生命周期、系统拓扑、核心不变式等）时，检查 `docs/architecture/README.md` 与对应领域文档是否已同步，详情参见 `.agents/skills/arch-align`。

---

## 4. 拆分无关改动

不相关领域的改动必须拆分多次提交（文档与功能、构建配置与业务代码分开提交）。

---

## 5. 提交信息格式

格式：`type(scope): 标题`


- **标题用中文短句**、不加句号（硬性要求：必须包含至少 2 个汉字，严禁写纯英文）。
  - ❌ 严禁：`feat(cli): add new flags`
  - ✅ 必须：`feat(cli): 新增参数支持`
- **type/scope 用英文**：
  - 常用 type：`feat` `fix` `docs` `style` `refactor` `perf` `test` `build` `ci` `chore` `revert`
  - scope 参考项目 `commitlint.config.js` 的 scope-enum 白名单，无合适则省略。


---

## 6. 提交与 Push 执行（非交互标准）

```bash
# 非交互式提交（必须携带 -m）
git commit -m "<type>(<scope>): <标题>"

# 提交后立即 push，防止工作丢失
git push

# 远端若有分叉，变基拉取后再 push
git pull --rebase --autostash
git push
```

---

## 7. Amend（修正红线）

仅当用户明确要求、且是刚提交未 push、无人依赖的最新提交时方可执行：

```bash
# 必须带 --no-edit 或显式 -m，严禁弹出编辑器
git commit --amend --no-edit
git push
```
