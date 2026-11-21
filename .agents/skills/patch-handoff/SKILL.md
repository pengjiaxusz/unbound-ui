---
name: patch-handoff
description: >-
  验收并合入离线补丁交接包：按序 `git am` 落提交、修编译与测试、把修复折回引入缺陷的提交、作者归一、强推与 issue 同步，并审计交接文档自身的验收声明。
  Triggers: 交接包, handoff 包, handoff.zip, 补丁集, patches, git am, 打补丁验收, patch series, 补丁包验收, 校验交接文档, issue 同步.
---

<!-- PENGJ_TEMPLATE_START -->

# 补丁交接包验收与合入 (Patch Handoff)

把上游 Agent 产出的离线交接包（`HANDOFF.md` + `patches/NNNN-*.patch`）真正落到 `unbound-ui` 并跑通验收。

**默认假设文档不可信**：这类包的 `HANDOFF.md` 常声称 `git am` / `just check` / `just test` 全部通过，实测常见「打完根本编译不过」且基线 SHA 写错。文档里的验收声明一律当作待验证假设。

## 快速开始

```bash
cd "E:/D/pengj/下游渲染工具/submodules/unbound-ui"
git status -sb; git rev-list --count origin/main..HEAD      # 必须干净、且为 0
git fetch origin
mkdir -p /tmp/handoff && unzip -o -q <包路径> -d /tmp/handoff
git am /tmp/handoff/patches/*.patch

just gate-check && just gate-lang && just fmt-check && just check && just test   # 全跑，不跳步
```

**`just check` 只跑 clippy，不含格式检查**。CI 的 `gate.yml` 是「提交标题 → 代码语言守卫 → 文案语言 → `cargo fmt --check`」四步，本地漏跑 `fmt-check` 就会在推送后才挂。收工前务必补 `cargo fmt`。

**推送前已由 `pre-push` 钩子兜底**：`git am` / `cherry-pick` / `rebase` 都不触发 `pre-commit`，只有 `pre-push` 拦得住。它自动跑 `cargo fmt --check` + 全树语言守卫 + `prepush.commands`（本项目 = `just gate-lang` + `just check`）。推送时看到 `[gate] pre-push:` 那几行，才算钩子真的生效了。

**钩子是每个克隆自己的本地配置**：`core.hooksPath` 不随 clone 传播。在临时克隆里干活必须先 `sh .githooks/install.sh`，否则 `pre-push` 等于不存在。

## 铁律

1. `HANDOFF.md` 与 `handoff/` 目录**永不入库**，只有 `patches/*.patch` 可经 `git am` 进入仓库。
2. 文档里的基线 SHA 不可信：以「能干净 `git am` 且 API 全对得上」的提交为真实基线，并在报告中指出文档错误。
3. 一张子工单对应一个提交；修复折回引入缺陷的提交，不追加 `fix(...)` 提交破坏映射。
4. 作者只能是 `pengjiaxusz <pengjiaxu.sz@gmail.com>`，包内自带的 `dev@unbound-ui.local` 必须重写。
5. 删除任何路径前先输出清单并获得确认，只删自己创建的，禁止通配符。
6. 测试失败先反馈用户，不擅自改断言。
7. 临时克隆先 `sh .githooks/install.sh` 再干活；`git push` 必须能看到 `[gate] pre-push:` 输出，否则钩子没生效。

## 工作流

| 步 | 动作 | 要点 |
| :-- | :--- | :--- |
| S1 | 落提交 | `git am patches/*.patch`；失败先判基线问题，再 `--show-current-patch` |
| S2 | 修编译 | `gate-check` → `gate-lang` → `fmt-check` → `check` → `test`；语法错会掩盖类型错，修完语法错必须重跑 |
| S3 | 折回提交 | 文件仅被单个提交触碰时，重放序列 + 覆盖文件 + `--amend`（用 `scripts/replay-patch-series.sh`） |
| S4 | 作者与推送 | `filter-branch --env-filter` 改作者（保留 author date）→ `push --force-with-lease` |
| S5 | 同步本地 | 强推后 `fetch --prune --force` 再 `reset --hard`，**不要 `git pull`** |
| S6 | issue 同步 | 逐条 `gh issue close` + 汇总评论 + `status:open` → `status:closed` |
| S7 | 清理 | 只删临时克隆、解包目录与日志，保留用户原始输入 |

S3 的折叠命令（可直接跑）：

```bash
.agents/skills/patch-handoff/scripts/replay-patch-series.sh /tmp/handoff/patches /tmp/fix-spec origin/main
```

`/tmp/fix-spec/0003/<仓库内相对路径>` 表示「补丁 0003 应用后用该文件整份覆盖再 amend」。折叠前必须确认 `git log --oneline <base>..HEAD -- <file> | wc -l` 为 1。

## 验收判据

| 命令 | 期望 |
| :--- | :--- |
| `just gate-check` | `[lang-guard] passed: N Rust file(s)` |
| `just gate-lang` | `通过：文案与语言轴一致。` |
| `just fmt-check` | 无 `Diff in` 输出（有则 `cargo fmt` 后重跑） |
| `just check` | `Finished` 且无 warning / error |
| `just test` | 全部 `test result: ok`，`failed=0`（2026-11-21 基线 2285 passed） |
| `pengj-templates-cli gate hook pre-push` | 先见 `[gate] pre-push: cargo fmt --check`，最后 `[gate] pre-push passed` |

再抽查新增控件的测试是否真被执行：`grep workbench_extension_widgets_keep_stable_shape <log>`。

## 测试覆盖不到的坑

编译与测试全绿 ≠ 功能完整。"24 态稳定形状"只比对图元**数量**，发现不了「该画的文字没画」——曾出现 `emit_preset_number_popup` 给 `payloads` 预留容量却从不 push 文案，菜单视觉上是空行。除跑测试外须抽查新增发射器是否真的画出文字/图标/分隔符，并写进报告交用户决定。

## 高级功能

报错到真实 API 的对照表、借用冲突标准解法、issue 英文模板与汇报格式，见 [REFERENCE.md](REFERENCE.md)。
