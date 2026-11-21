---
name: patch-handoff
description: >-

  验收并合入离线补丁交接包（`HANDOFF.md` + `patches/NNNN-*.patch`）：按序 `git am` 落提交、修编译与测试、把修复折回引入缺陷的提交、作者归一、强推、同步工作树并关闭对应 issue。交接文档里的每一条验收声明，在你亲手复现之前都是未验证假设。当收到别的 agent 产出的补丁集/交接 zip，或交付的补丁集并不像文档说的那样能编译时使用。 Triggers: patch-handoff, 交接包, handoff 包, handoff.zip, 补丁集, patches, git am, 打补丁验收, patch series, 补丁包验收, 校验交接文档.

---

<!-- PENGJ_TEMPLATE_START -->
# 补丁交接包验收与合入（Patch Handoff）

把别的 agent 产出的离线交接包，变成真正落在仓库里的提交，并用项目真实门禁证明它成立——而不是用文档的声称。

**默认文档不可信，直到你复现它。** 这类包常声称 `git am` / lint / 测试全部通过，实测常见「打完根本编译不过」且基线 SHA 写错。文档里的每条声明都当假设，逐条复现，把结论写进报告。

## 动手前先钉六个参数

参数一律从仓库读，不从 `HANDOFF.md` 读。

| 参数 | 取值处 | 为什么必须钉 |
| :--- | :--- | :--- |
| 基线 ref | `git rev-parse --abbrev-ref origin/HEAD`，并与文档声称的 SHA 对照 | `git am` 失败多数是基线不对，不是补丁坏 |
| 验收命令链 | `justfile` / `Makefile` / `package.json` / `.github/workflows/*.yml` | 你自选的命令不等于 CI 门禁 |
| 格式检查位置 | CI 是否有独立的 `fmt --check` / `lint` 步骤 | linter 过了不代表格式过 |
| 钩子安装 | `git config core.hooksPath`；新克隆要跑仓库的安装脚本 | 钩子是每克隆自己的配置，不随 clone 传播 |
| 作者身份 | `git config user.email` 或仓库明文约定 | 包里的作者多为合成占位身份 |
| issue 仓库 | 文档引用的编号 + 仓库名 + 状态标签 | 关单是落地的一部分，不是收尾补充 |

## 快速开始

```bash
PKG=<交接 zip 或已解包目录>
BASE="origin/$(git rev-parse --abbrev-ref origin/HEAD | sed 's|^origin/||')"
git status -sb; git rev-list --count "$BASE"..HEAD     # 必须干净且为 0

.agents/skills/patch-handoff/scripts/inspect-handoff-package.sh "$PKG"   # 落库前体检
.agents/skills/patch-handoff/scripts/replay-patch-series.sh <dir>/patches <fix-spec-dir> "$BASE"

# 然后整条门禁链全跑，不跳步
```

## 工作流

| 步 | 动作 | 要点 |
| :-- | :--- | :--- |
| S1 | 体检 | 清点补丁、作者、涉及文件；把文档声称的基线与真实基线对照 |
| S2 | 落提交 | 按序 `git am patches/*.patch`；失败先判基线，再 `--show-current-patch` |
| S3 | 修编译 | 自上而下跑门禁链；语法错会掩盖类型错，每修完一类必须重跑 |
| S4 | 折回 | 把修复折进引入缺陷的那条提交，保持一单一提交 |
| S5 | 作者与推送 | 归一作者（保留 author date），再 `push --force-with-lease` |
| S6 | 同步 | 强推后 `fetch --prune --force` 再 `reset --hard`，**不要** `git pull` |
| S7 | issue | 逐条关闭已实现工单并写单条摘要评论，再翻状态标签 |
| S8 | 清理 | 只删自己创建的：解包目录、日志、临时克隆 |

## 铁律

1. `HANDOFF.md` 与交接目录**永不入库**，只有 `patches/*.patch` 可经 `git am` 进入仓库。
2. 文档里的基线 SHA 不是证据：真实基线是「补丁能干净应用且 API 全对得上」的那个提交，差异要写进报告。
3. 一张子工单对应一个提交；修复折回引入缺陷的提交，不追加 `fix(...)` 提交破坏 1:1 映射。
4. 作者必须是仓库的真实身份，包内自带的合成身份必须重写。
5. 删除任何路径前先输出清单并获得确认，只删自己创建的，禁止通配符。
6. 测试失败先反馈用户，不擅自改断言把失败糊过去。
7. `git am` / `cherry-pick` / `rebase` 都不触发 `pre-commit`，只有 `pre-push` 拦得住；临时克隆里先装钩子再干活。

## 验收判据怎么写

按项目真实门禁链列一张两列表——命令 / 通过标志（在下方项目专属区或引用文件里填）。然后守住三条边界：

- 本地门禁通过只说明你跑的那几条命令过了，不等于 CI 过。CI 通常是超集（格式、语言守卫、提交标题）。
- 测试看汇总行（`failed=0` 与通过数），不要只看没有红字。
- 新增行为要在日志里 grep 到具体测试名，确认它真被执行，而不是被跳过或没命名。

## 测试覆盖不到的地方

编译与测试全绿 ≠ 功能完整。只数元素个数的形状/快照断言，分不清「元素在」和「元素什么都没画」——真实案例里某弹窗给菜单文案预留了容量却从不 push，整列视觉上是空行，测试全过。除跑测试外须亲手抽查新增输出路径（文字、图标、分隔符、副作用），写进报告交用户决定。

## 脚本

| 脚本 | 用途 |
| :--- | :--- |
| `scripts/inspect-handoff-package.sh` | 落库前体检：补丁清单、作者、涉及文件、声称基线与实际基线 |
| `scripts/replay-patch-series.sh` | 按序 `git am`，并把修复文件折回对应提交 |
| `scripts/normalize-authors.sh` | 在提交区间内重写作者，保留 author date |
| `scripts/sync-after-force-push.sh` | 强推后安全同步工作树，脏树或有未推送提交时拒绝执行 |

## 参考

- [references/handoff-audit.md](references/handoff-audit.md) —— 声明分类、常见谎报、报告格式
- [references/git-mechanics.md](references/git-mechanics.md) —— 钩子触发面矩阵、`git am` 失败诊断、折叠前提、作者重写
- [references/field-notes.md](references/field-notes.md) —— 一次真实落地的「报错 → 真实 API」对照
<!-- PENGJ_TEMPLATE_END -->


## 本项目（unbound-ui）专属参数

上面托管块里的「六个参数」，在本项目的取值如下。仓库约定、控件登记、issue 同步模板等更细的条目见 [REFERENCE.md](REFERENCE.md)。

### 基线、身份、issue 仓库

| 参数 | 本项目取值 |
| :--- | :--- |
| 基线 ref | `origin/main`（子模块自己的 `main`，不是父仓的 `dev`） |
| 作者身份 | `pengjiaxusz <pengjiaxu.sz@gmail.com>`；包内自带的 `dev@unbound-ui.local` 必须重写 |
| issue 仓库 | `pengjiaxusz/unbound-ui`，状态标签 `status:open` → `status:closed` |
| 语言守卫 | `mode=english`：中文 `///` 文档注释可用，新增公开 API 注释风格与邻居一致 |

### 门禁链与通过标志

```bash
just gate-check && just gate-lang && just fmt-check && just check && just test   # 全跑，不跳步
```

| 命令 | 通过标志 |
| :--- | :--- |
| `just gate-check` | `[lang-guard] passed: N Rust file(s)` |
| `just gate-lang` | `通过：文案与语言轴一致。` |
| `just fmt-check` | 无 `Diff in` 输出（有则 `cargo fmt` 后重跑） |
| `just check` | `Finished` 且无 warning / error |
| `just test` | 全部 `test result: ok`，`failed=0`（2026-11-21 基线 2285 passed） |
| 推送时 | 必须看到 `[gate] pre-push:` 那几行，否则钩子没生效 |

**`just check` 只跑 clippy，不含格式检查。** CI 的 `gate.yml` 是「提交标题 → 代码语言守卫 → 文案语言 → `cargo fmt --check`」四步，漏跑 `fmt-check` 会到推送后才挂。`git am` / `cherry-pick` / `rebase` 都不触发 `pre-commit`，只有 `pre-push` 兜得住；临时克隆先 `sh .githooks/install.sh`。

### 测试覆盖不到的那类缺口（本项目实例）

「24 态稳定形状」只比对图元**数量**，发现不了「该画的文字没画」——曾出现 `emit_preset_number_popup` 给 `payloads` 预留容量却从不 push 文案，菜单视觉上是空行而测试全绿。除跑测试外须抽查新增发射器是否真的画出文字 / 图标 / 分隔符，并写进报告交用户决定。
