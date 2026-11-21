# 补丁交接包处置参考

记录 2026-11-21 处理 Issue #114 交接包（12 补丁 / 12 子工单）时遇到的真实报错与改法。同类包大概率重复同类错误，可按表速查。

## 一、错误 → 真实 API

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

## 二、借用冲突的固定解法

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

## 三、clippy 会拦的细节

- `let mut payloads = Vec::with_capacity(..)` 之后只被 move 进结构体、从不 push → 去掉 `mut`。

## 四、折叠修复的前提

`cp 最终版 → git add → commit --amend` 仅在该文件于整个序列中被单次修改时成立：

```bash
git log --oneline <base>..HEAD -- crates/unbound-widgets/src/<file>.rs | wc -l   # 必须为 1
```

大于 1 时不要折叠，改为在末条提交单独修，并在报告中说明映射为何被破坏。

## 五、作者规整

```bash
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --env-filter \
  'export GIT_AUTHOR_NAME="pengjiaxusz"; export GIT_AUTHOR_EMAIL="pengjiaxu.sz@gmail.com"' \
  <base>..HEAD
git log --format="%an <%ae>" <base>..HEAD | sort -u     # 必须只剩一行
```

`--env-filter` 只改作者、保留 author date；不要用 `--reset-author`（会连带改时间）。强推前确认 `<base>` 是远端旧 HEAD 的父提交。

## 六、强推后同步本地工作树

```bash
cd "E:/D/pengj/下游渲染工具/submodules/unbound-ui"
git fetch --prune --force origin            # 强推后必须加 --force，否则 origin/main 仍是旧值
git rev-list --count origin/main..HEAD      # 必须为 0，否则停手问用户
git status --porcelain -uall                # 必须为空
git reset --hard origin/main
```

父仓 下游渲染工具（`dev`）会显示 ` M submodules/unbound-ui`，因其记录的指针本就比子模块实际 checkout 更旧；这是用户既有状态，不要替他提交或回滚父仓。

## 七、issue 同步模板（英文，与仓库历史一致）

```bash
REPO="pengjiaxusz/unbound-ui"
gh issue close 102 -R "$REPO" -c "Implemented ScaleOsd state machine, OverlayStack mounting, AccessKit Live::Polite semantics, and headless geometry tests."
gh issue comment 114 -R "$REPO" -b "All 12 tickets (#102-#113) have been implemented as individual commits (<base>..<head>), verified locally with \`just check\` and \`just test\` (<N> passed / 0 failed), and pushed to \`main\`.

Note: the offline patch series did not compile as delivered (<K> hard errors). The fixes were folded back into the commits that introduced each defect, so each commit still maps 1:1 to its ticket."
for n in $(seq 102 113); do gh issue edit $n -R "$REPO" --remove-label "status:open" --add-label "status:closed"; done
```

必须如实披露原本编译不过及修了哪些点，不要只写「已实现并验收通过」。

## 八、仓库约定

- 语言守卫 `mode=english`：中文 `///` 文档注释可用（实测通过），新增公开 API 注释风格与邻居一致即可。
- `just test` 会先 `cargo build -p app-guest`（`host_roundtrip` 要加载真实 cdylib），跳过在干净环境会硬失败。
- `CARGO_TARGET_DIR` 由 `pengj-templates-cli worktree target-dir --dir .` 推导到仓库外共享目录；正因如此，另克隆一份仓库会连带数十 GB 构建产物，处置完必须清理。
- 新增控件需登记 `catalog.rs` 的 `probe_workbench_widgets` 与 `interactive_tests::workbench_extension_widgets_keep_stable_shape_across_24_states`（3 phase × 2 focused × 2 disabled × 2 selected）。
- 无障碍子控件 id 用 `xxx_node(node, idx)` 生成，命中区与 `A11yNode` 必须成对注册（`WidgetOutput::with_sub_control`）。

## 九、汇报格式

最终回复必须含四块：① 在哪个工作树、什么顺序、提交区间；② 逐条列「文件 : 问题 : 修法」；③ 三条命令的结论与测试通过数；④ 遗留问题（测试覆盖不到的功能缺口、文档写错的基线 SHA）。
