# unbound-ui 补丁交接档案

本文件是 [SKILL.md](SKILL.md) 的项目档案：把那个技能里「按仓库现读」的部分，在本仓库的确定取值固定下来。技能本体是通用的（由 `pengj-templates` 下发），托管块之外只放这里的内容。

记录来源：2026-11-21 处理 Issue #114 交接包（12 补丁 / 12 子工单）的实跑。

## 一、落地后的固定动作

```bash
# 折叠修复（单次触碰前提由脚本自行校验，违反即中止）
.agents/skills/patch-handoff/scripts/replay-patch-series.sh /tmp/handoff/patches /tmp/fix-spec origin/main
# /tmp/fix-spec/0003/<仓库内相对路径> 表示补丁 0003 应用后用该文件整份覆盖再 amend

# 作者归一（保留 author date）
.agents/skills/patch-handoff/scripts/normalize-authors.sh origin/main pengjiaxusz pengjiaxu.sz@gmail.com

# 强推后同步工作树（不要 git pull）
.agents/skills/patch-handoff/scripts/sync-after-force-push.sh origin main
```

强推前的等价手写序列（脚本就是这么做的）：

```bash
git fetch --prune --force origin        # 强推后不加 --force，origin/main 仍是旧值
git rev-list --count origin/main..HEAD  # 必须为 0，否则停手问用户
git status --porcelain -uall            # 必须为空
git reset --hard origin/main
```

父仓（`dev`）会显示 ` M submodules/unbound-ui`，因其记录的指针本就比子模块实际 checkout 更旧；这是既有状态，不要替他提交或回滚父仓。

## 二、包里的写法 → 本仓库真实 API

同类包大概率重复同类错误，可按表速查。

| 包里的写法 | 仓库真实 API | 报错 |
| :--- | :--- | :--- |
| `b'\'` | `b'\\'` | `character literal may only contain one codepoint` + `unterminated character literal` |
| `fn byte_prefix(text: &str, ...) -> &str` | 加 `<'a>` 绑定到 `text` | `missing lifetime specifier` |
| `style.size` | `style.font_size`（`unbound_text::TextStyle`） | `no field 'size'` |
| `ValidationState::Normal` | `ValidationState::None`（变体 `None`/`Error`/`Warning`） | `no variant named 'Normal'` |
| `Rect::contains(p)` | `Rect::contains_point(p)` / `contains_rect(&r)` | `no method named 'contains'` |
| `place_anchored_popup_detailed(anchor, size, placement, viewport, gap, offset)` | `(anchor: Rect, popup_size: Vec2, viewport: Rect, placement: Placement)`，无 gap/offset 形参 | `takes 4 arguments but 6 were supplied` |
| 外层 `#[derive(Eq)]` 含 `TextEditState` | `TextEditState` 仅有 `PartialEq`，外层去掉 `Eq` | `TextEditState: Eq is not satisfied` |

判定原则：报错集中为「方法/字段/变体不存在」时，按仓库真实 API 改，**不要**给公共类型补方法；`trait bound` 报错优先去掉新类型的多余派生，不要动共享类型。

## 三、借用冲突的固定解法

`CodeEditorState::split_lines()` 返回 `Vec<&str>`，借用 `self.source`；借用存活期间改 `self.cursor_*` 即报 `cannot assign to self.cursor_line because it is borrowed`。

```rust
// 反例
let lines = self.split_lines();
self.cursor_line -= 1;
self.cursor_col = Self::line_char_len(lines[self.cursor_line]);

// 正解：块内取标量，借用结束再赋值
let len = {
    let lines = self.split_lines();
    Self::line_char_len(lines[target])
};
self.cursor_line = target;
self.cursor_col = self.cursor_col.min(len);
```

`move_right` 需同时用「当前行长 + 总行数」，在块内返回元组 `(len, count)`。

## 四、clippy 会拦的细节

- `let mut payloads = Vec::with_capacity(..)` 之后只被 move 进结构体、从不 push → 去掉 `mut`。
- 折叠修复的前提仍是单次触碰：`git log --oneline <base>..HEAD -- crates/unbound-widgets/src/<file>.rs | wc -l` 必须为 1，大于 1 改为末条提交单独修并在报告里说明映射为何被破坏。

## 五、仓库约定

- **`git am` 不触发 `pre-commit`**：实测触发面是——`git commit`（含 `--amend`）触发 `pre-commit` + `commit-msg`；`git am` 只触发 `pre-applypatch`/`post-applypatch`；`git cherry-pick` 与 `git rebase` 一个都不触发。所以补丁集落下的提交天然没被 rustfmt 扫过。
- **补救靠 `pre-push`，不是「记得手动补充」**：`pre-push` 覆盖所有造历史方式，gate 层已内置（`cargo fmt --check` + 全树语言守卫 + `prepush.commands`）。`pre-applypatch` 不要拿来修补：实测在钩子里改文件 + `git add` 进不了提交（`git am` 提交后会 `read-tree` 冲掉索引，只留一棵脏工作树），它只能拦、不能改。
- **钩子是每克隆配置**：`core.hooksPath` 存在 `.git/config`，不随 clone 传播。临时克隆里没跑 `sh .githooks/install.sh` 就等于没有任何钩子——2026-11-21 那轮正是在临时克隆里推的，`pre-push` 当时还不存在，两重原因叠加才让 12 条未格式化的提交推到 CI。
- **本地验收 ≠ CI 门禁**：`gate.yml` 只跑「提交标题 → 代码语言守卫 → 文案语言 → `cargo fmt --check`」，而 `just check` 是 clippy、不含格式检查。批量新增模块后 `pub use` 的排序与长行会被 rustfmt 重排，漏跑 `fmt-check` 必然在 CI 挂（2026-11-21 即因此连挂两次）。
- 语言守卫 `mode=english`：中文 `///` 文档注释可用（实测通过），新增公开 API 注释风格与邻居一致即可。
- `just test` 会先 `cargo build -p app-guest`（`host_roundtrip` 要加载真实 cdylib），跳过在干净环境会硬失败。
- `CARGO_TARGET_DIR` 由 `pengj-templates-cli worktree target-dir --dir .` 推导到仓库外共享目录；正因如此，另克隆一份仓库会连带数十 GB 构建产物，处置完必须清理。
- 新增控件需登记 `catalog.rs` 的 `probe_workbench_widgets` 与 `interactive_tests::workbench_extension_widgets_keep_stable_shape_across_24_states`（3 phase × 2 focused × 2 disabled × 2 selected）。
- 无障碍子控件 id 用 `xxx_node(node, idx)` 生成，命中区与 `A11yNode` 必须成对注册（`WidgetOutput::with_sub_control`）。

## 六、issue 同步模板（英文，与仓库历史一致）

```bash
REPO="pengjiaxusz/unbound-ui"
gh issue close 102 -R "$REPO" -c "Implemented ScaleOsd state machine, OverlayStack mounting, AccessKit Live::Polite semantics, and headless geometry tests."
gh issue comment 114 -R "$REPO" -b "All 12 tickets (#102-#113) have been implemented as individual commits (<base>..<head>), verified locally with \`just check\` and \`just test\` (<N> passed / 0 failed), and pushed to \`main\`.

Note: the offline patch series did not compile as delivered (<K> hard errors). The fixes were folded back into the commits that introduced each defect, so each commit still maps 1:1 to its ticket."
for n in $(seq 102 113); do gh issue close $n -R "$REPO"; done
```

开关状态一律用 `gh issue close` / `gh issue reopen` 表达，不要新增 `status:*` 标签——GitHub 自带的 Open/Closed 状态已经够了，重复打标只会让列表眼花。

必须如实披露原本编译不过及修了哪些点，不要只写「已实现并验收通过」。

## 七、汇报格式

最终回复必须含四块：① 在哪个工作树、什么顺序、提交区间；② 逐条列「文件 : 问题 : 修法」；③ 门禁链的结论与测试通过数；④ 遗留问题（测试覆盖不到的功能缺口、文档写错的基线 SHA）。
