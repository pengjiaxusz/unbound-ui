---
name: patch-handoff
description: >-

  验收并合入离线补丁交接包（`HANDOFF.md` + `patches/NNNN-*.patch`）：按序 `git am` 落提交、修编译与测试、把修复折回引入缺陷的提交、作者自动归一为仓库拥有者身份（禁止手填）、强推、同步工作树；验收中发现的问题当场上 GitHub issue，交接完成后逐条收口 issue 状态，并删掉解包出来的临时内容。交接文档里的每一条验收声明，在你亲手复现之前都是未验证假设。当收到别的 agent 产出的补丁集/交接 zip，或交付的补丁集并不像文档说的那样能编译时使用。 Triggers: patch-handoff, 交接包, handoff 包, handoff.zip, 补丁集, patches, git am, 打补丁验收, patch series, 补丁包验收, 校验交接文档, 交接建单, issue 收口.

---

<!-- PENGJ_TEMPLATE_START -->
# 补丁交接包验收与合入（Patch Handoff）

把别的 agent 产出的离线交接包，变成真正落在仓库里的提交，并用项目真实门禁证明它成立——而不是用文档的声称。

**默认文档不可信，直到你复现它。** 这类包常声称 `git am` / lint / 测试全部通过，实测常见「打完根本编译不过」且基线 SHA 写错。文档里的每条声明都当假设，逐条复现，把结论写进报告。

## 就地验收，不要克隆项目来测试

**直接在用户现有的工作区里验收并落补丁，不要为了「安全测试」再 `git clone` 一份、拷到临时目录或另起一套并行构建。** 复杂项目重新克隆加重装依赖动辄几十分钟到几小时，收益为零：要复现的门禁就在这个工作区里，提交也必须落在本地这份仓库。另起一份只会多出一个需要事后对齐的分叉状态，徒增工作量与风险。

只有用户明确要求隔离时（例如工作区要为另一个进行中的任务保持原样）才另起克隆，且即便如此也要先在克隆里装好钩子再干活。默认就是：在当前工作区操作、保持工作树干净到能 `git am`、只清理自己创建的解包目录和日志（见 S8）。

## 动手前先钉七个参数

参数一律从仓库读，不从 `HANDOFF.md` 读。

| 参数 | 取值处 | 为什么必须钉 |
| :--- | :--- | :--- |
| 基线 ref | `git rev-parse --abbrev-ref origin/HEAD`，并与文档声称的 SHA 对照 | `git am` 失败多数是基线不对，不是补丁坏 |
| 验收命令链 | `justfile` / `Makefile` / `package.json` / `.github/workflows/*.yml` | 你自选的命令不等于 CI 门禁 |
| 格式检查位置 | CI 是否有独立的 `fmt --check` / `lint` 步骤 | linter 过了不代表格式过 |
| 钩子安装 | `git config core.hooksPath`；在当前工作区跑仓库的安装脚本（若未装） | 钩子是每个工作区自己的配置，不随 clone 传播 |
| 作者身份 | `normalize-authors.sh --print-identity` —— owner 取自 `origin` URL + `git config user.email`；**禁止手填或从包里读** | 包里的作者多为合成占位身份，手填名字就是乱填的源头 |
| issue 仓库 | 文档引用的编号 + 仓库名 | 关单是落地的一部分，不是收尾补充 |
| issue 写入目标 | `gh repo view --json nameWithOwner`；编号在本仓库是否真存在（`gh issue view <n>`） | fork 里 `gh` 默认指向上游仓库，发错仓等于白发 |
| issue 状态流 | `gh label list`；项目现有的在办 / 待定标签 | 不知道翻哪个标签，结果就是只关单不翻标签，看板从此失真 |

## 快速开始

```bash
PKG=<交接 zip 或已解包目录>
BASE="origin/$(git rev-parse --abbrev-ref origin/HEAD | sed 's|^origin/||')"
git status -sb; git rev-list --count "$BASE"..HEAD     # 必须干净且为 0

.agents/skills/patch-handoff/scripts/inspect-handoff-package.sh "$PKG"   # 落库前体检
.agents/skills/patch-handoff/scripts/normalize-authors.sh "$BASE" --print-identity  # 确认归一目标身份
.agents/skills/patch-handoff/scripts/replay-patch-series.sh <dir>/patches <fix-spec-dir> "$BASE"

# 然后整条门禁链全跑，不跳步
# 折回修复后归一作者，再推送：
.agents/skills/patch-handoff/scripts/normalize-authors.sh "$BASE"   # 不传名字/邮箱：自动从仓库解析
```

## 工作流

| 步 | 动作 | 要点 |
| :-- | :--- | :--- |
| S1 | 体检 | 清点补丁、作者、涉及文件；把文档声称的基线与真实基线对照；确认的每处不符当场上 issue（先查重） |
| S2 | 落提交 | 按序 `git am patches/*.patch`；失败先判基线，再 `--show-current-patch` |
| S3 | 修编译 | 自上而下跑门禁链；语法错会掩盖类型错，每修完一类必须重跑；修不掉的立刻建单 |
| S4 | 折回 | 把修复折进引入缺陷的那条提交，保持一单一提交 |
| S5 | 作者与推送 | **不传名字/邮箱**运行 `normalize-authors.sh "$BASE"`，身份自动从仓库解析（保留 author date），再 `push --force-with-lease` |
| S6 | 同步 | 强推后 `fetch --prune --force` 再 `reset --hard`，**不要** `git pull` |
| S7 | issue 收口 | **两条线都要收**：逐条关闭已实现工单（先评论、再关闭、后翻标签），以及 S1–S4 建的问题单（已修的贴折回 SHA 关闭，未修的带标签留开并指明下一步归谁） |
| S8 | 清理 | 删掉自己创建的临时内容：解包目录、fix-spec 目录、日志，逐个验证已消失。不应存在临时克隆——不要为了验收而克隆项目 |

## 交接时发现的问题，直接落成 issue

报告里的文字只会被读一遍，然后烂掉；issue 有个编号，谁都能指着它说话。验收过程中确认的每一处问题，**在发现它的那一轮就建单**，不要攒：

- 交付的补丁集编译不过，或文档声称的结论复现不出来。
- 范围缺口：声称实现了但补丁里没有，或文档点名的工单没有对应补丁。
- 测试看不见的行为缺口，而用户还没拍板。
- 你主动决定不修的任何东西。

建单规矩：

1. **先查重。** `gh issue list --state all --search '"<症状>"'`；已有同因的单就补一条评论，不要开第二张。
2. **一个缺陷一张单。** 标题 `[handoff] <包名>: <一句话症状>`；正文写清：包名 + 补丁编号、声称 vs 实测、能复现的命令、什么算修好。
3. **用项目已有标签**（`gh label list`），不要为一次交接自造一套标签体系。
4. **报告里回指编号。** 每条遗留行都要带 `#编号`——没有编号的那一行，就是你忘了建的单。
5. **没有 `gh`、没有写权限、或 `gh` 指向 fork 的上游仓库**：在报告里明说，并把你本该执行的命令原样贴出来。绝不允许静默退回成纯文字。

建单不是装饰：S7 必须给每一张单一个终态。

## issue 收口 —— 交接完成后，一条都不许悬着

两条线都跑，每条都要落到一个你说得出的状态：

1. **工单线**（文档映射的 ticket）：逐条 `gh issue comment <n>` 写落地摘要——折回提交 SHA、验收命令、测试通过数——**然后** `gh issue close <n>`，**最后**翻标签（先加新标签再撤旧标签，中间不留无标签态）。不要用 `gh issue close --comment`：对已关闭的 issue 它会丢掉评论却仍返回 0。
2. **问题线**（你在 S1–S4 建的单）：本轮已修掉的 → 评论贴折回 SHA 再关闭；没修、或在等用户决定的 → 保持 open，补上项目的待定/阻塞标签，指派或 @ 到下一步的负责人，并在报告里写明。
3. **自检。** `gh issue list --state open --search '[handoff]'` 剩下的必须只剩你解释得清的那几张。报告收尾固定三行：`已关闭 #…` / `新建 #…` / `仍开 #…，原因`。

## 清理 —— 解包出来的临时内容必须消失

交接包里只有 `patches/*.patch` 有长期价值；`HANDOFF.md`、文档附带文件、你自己的 `fix-spec/` 都是脚手架。交接落完之后删干净：

- 解包目录（`handoff-*/`、`mktemp -d` 出来的路径）、fix-spec 目录、日志、导出中间文件。
- 删之前先 `ls` 列清单（铁律 5），只删自己创建的，禁通配符，不碰 `~/.cache` 这类共享位置。
- 例外：`git am` 中途停下留下的半应用工作树是证据——留着交用户看，用户确认后再删。
- 收尾校验：`git status --porcelain -uall` 为空，解包目录确认已不存在。留一个解包目录，下一份交接包就会和上一份搅在一起。

## 铁律

1. `HANDOFF.md` 与交接目录**永不入库**，只有 `patches/*.patch` 可经 `git am` 进入仓库。
2. 文档里的基线 SHA 不是证据：真实基线是「补丁能干净应用且 API 全对得上」的那个提交，差异要写进报告。
3. 一张子工单对应一个提交；修复折回引入缺陷的提交，不追加 `fix(...)` 提交破坏 1:1 映射。
4. 作者必须是仓库拥有者的身份：**不传名字/邮箱**运行 `normalize-authors.sh`，由脚本从 `origin` 解析 owner、从 `git config user.email` 取邮箱；绝不手填，也不从交接包里抄。
5. 删除任何路径前先输出清单并获得确认，只删自己创建的，禁止通配符。
6. 测试失败先反馈用户，不擅自改断言把失败糊过去。
7. `git am` / `cherry-pick` / `rebase` 都不触发 `pre-commit`，只有 `pre-push` 拦得住；干活前先确认当前工作区钩子已装（若被明确要求隔离到克隆，则先在克隆里装钩子）。
8. **在用户的工作区里干活。** 绝不为验收或落补丁而克隆、复制项目——复杂项目重新克隆加重装依赖要耗掉几十分钟到几小时，收益为零。
9. **发现了却没建单的问题等于不存在。** 每处确认的缺陷都在发现它的那一轮进 GitHub issue（先查重、带复现命令），报告里引用编号。
10. **落地后范围内的 issue 不许悬着。** 工单与你建的单，一律以「带折回 SHA 的关闭」或「带标签 + 明确责任人的留开」收尾；`gh issue close --comment` 不能替代「先评论再关闭」。
11. **解包临时内容不是产物。** 解包目录、fix-spec、日志在 S8 删除，并且要验证已消失，而不是假设。

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
| `scripts/normalize-authors.sh` | 在提交区间内重写作者为自动解析的仓库拥有者身份（保留 author date）；`--print-identity` 预览目标身份 |
| `scripts/sync-after-force-push.sh` | 强推后安全同步工作树，脏树或有未推送提交时拒绝执行 |

## 参考

- [references/handoff-audit.md](references/handoff-audit.md) —— 声明分类、常见谎报、报告格式
- [references/git-mechanics.md](references/git-mechanics.md) —— 钩子触发面矩阵、`git am` 失败诊断、折叠前提、作者重写
- [references/field-notes.md](references/field-notes.md) —— 一次真实落地的「报错 → 真实 API」对照
- [references/issue-closeout.md](references/issue-closeout.md) —— 何时建单、正文模板、查重、状态机、`gh` 的失败模式
<!-- PENGJ_TEMPLATE_END -->


## 本项目（unbound-ui）专属参数

上面托管块里的「六个参数」，在本项目的取值如下。仓库约定、控件登记、issue 同步模板等更细的条目见 [REFERENCE.md](REFERENCE.md)。

### 基线、身份、issue 仓库

| 参数 | 本项目取值 |
| :--- | :--- |
| 基线 ref | `origin/main`（子模块自己的 `main`，不是父仓的 `dev`） |
| 作者身份 | `pengjiaxusz <pengjiaxu.sz@gmail.com>`；包内自带的 `dev@unbound-ui.local` 必须重写 |
| issue 仓库 | `pengjiaxusz/unbound-ui`，开关状态用 `gh issue close` / `reopen`，不设 `status:*` 标签 |
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
