//! 命令面板（CommandPalette）：带排序命令列表的搜索框。
//!
//! 排序是 `(query, command, recency)` 的纯函数，因此相同查询总能产出相同顺序，便于测试断言。
//! 评分刻意保持粗粒度——前缀匹配优于子串匹配，子串匹配优于子序列匹配——
//! 最近使用度仅用于打破平局，绝不会提升不匹配项。

use accesskit::Role;
use rustc_hash::FxHashMap;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::focus_underline;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 默认行高。对齐 WinUI `MenuFlyoutThemeMinHeight = 32`（建议行与菜单行同档）。
pub const COMMAND_ROW: f32 = 32.0;

/// 搜索框高度。对齐 WinUI `TextControlThemeMinHeight = 32`（AutoSuggestBox / TextBox）。
pub const COMMAND_INPUT: f32 = 32.0;

/// 面板（浮层）圆角。对齐 WinUI `OverlayCornerRadius = 8`（FlyoutPresenter）。
const PALETTE_RADIUS: f32 = 8.0;
/// 搜索框圆角。对齐 WinUI `ControlCornerRadius = 4`。
const INPUT_RADIUS: f32 = 4.0;
/// 结果行圆角。对齐 WinUI `ListViewItemCornerRadius = 4` / `ControlCornerRadius = 4`。
const ROW_RADIUS: f32 = 4.0;
/// 高亮行左侧强调条宽度（沿用本组件既有的行标记语义）。
const ROW_MARKER_WIDTH: f32 = 3.0;

/// 面板中的单条命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// 稳定标识。
    pub id: u64,
    /// 列表中显示的标签。
    pub title: String,
    /// 分组标题。
    pub group: String,
    /// 参与匹配但不显示的额外关键词。
    pub keywords: Vec<String>,
    /// 显示在右侧的快捷键提示。
    pub shortcut: Option<String>,
}

impl Command {
    /// 创建带标题与分组的命令。
    #[must_use]
    pub fn new(id: u64, title: impl Into<String>, group: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            group: group.into(),
            keywords: Vec::new(),
            shortcut: None,
        }
    }

    /// 添加隐藏搜索关键词。
    #[must_use]
    pub fn with_keywords(mut self, keywords: &[&str]) -> Self {
        self.keywords = keywords.iter().map(|word| (*word).to_owned()).collect();
        self
    }

    /// 添加快捷键提示。
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

/// 命令面板的纯描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct CommandPaletteDesc {
    /// 面板矩形。
    pub rect: Rect,
    /// 查询为空时显示的占位文本。
    pub placeholder: String,
    /// 行高。
    pub row_height: f32,
    /// 单次最多显示的行数。
    pub max_rows: usize,
    /// 绘制顺序键。
    pub z: u16,
}

impl CommandPaletteDesc {
    /// 创建指定尺寸的命令面板。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            placeholder: "Type a command…".to_owned(),
            row_height: COMMAND_ROW,
            max_rows: 8,
            z: 900,
        }
    }

    /// 搜索框矩形。
    #[must_use]
    pub fn input_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x,
            self.rect.origin.y,
            self.rect.size.x,
            COMMAND_INPUT.min(self.rect.size.y),
        )
    }

    /// 搜索框下方的列表矩形。
    #[must_use]
    pub fn list_rect(&self) -> Rect {
        let input = self.input_rect();
        Rect::new(
            self.rect.origin.x,
            input.max().y,
            self.rect.size.x,
            (self.rect.size.y - input.size.y).max(0.0),
        )
    }

    /// 第 n 个可见结果的矩形。
    #[must_use]
    pub fn row_rect(&self, index: usize) -> Rect {
        let list = self.list_rect();
        Rect::new(
            list.origin.x,
            list.origin.y + index as f32 * self.row_height,
            list.size.x,
            self.row_height
                .min((list.max().y - list.origin.y - index as f32 * self.row_height).max(0.0)),
        )
    }
}

/// 命令与查询的匹配程度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// 不匹配。
    None,
    /// 字符按顺序出现，中间存在间隔。
    Subsequence,
    /// 查询字符串作为子串出现在标题或关键词中。
    Substring,
    /// 标题或关键词以查询字符串开头。
    Prefix,
}

/// 对单条命令与查询进行评分。
///
/// 无匹配时返回 `None`。最近使用度排名作为平局决胜分最多占 90 分，
/// 因此绝不会将子序列匹配提升到子串匹配之上。
#[must_use]
pub fn score_command(query: &str, command: &Command, recency: u32) -> Option<i32> {
    let query = query.trim().to_lowercase();
    let bonus = (recency.min(9) as i32) * 10;
    if query.is_empty() {
        return Some(bonus);
    }
    let title = command.title.to_lowercase();

    let mut best = MatchKind::None;
    let mut best_score = 0;
    for candidate in
        std::iter::once(title.as_str()).chain(command.keywords.iter().map(String::as_str))
    {
        let candidate = candidate.to_lowercase();
        let (kind, score) = if candidate.starts_with(&query) {
            (MatchKind::Prefix, 1000 - candidate.len() as i32)
        } else if let Some(position) = candidate.find(&query) {
            (MatchKind::Substring, 500 - position as i32)
        } else if let Some(gaps) = subsequence_gaps(&candidate, &query) {
            (MatchKind::Subsequence, 200 - gaps as i32)
        } else {
            (MatchKind::None, i32::MIN)
        };
        if kind > best {
            best = kind;
            best_score = score;
        } else if kind == best && kind != MatchKind::None {
            best_score = best_score.max(score);
        }
    }
    (best != MatchKind::None).then_some(best_score + bonus)
}

/// 当 `query` 是 `text` 的子序列时跳过的字符数。
fn subsequence_gaps(text: &str, query: &str) -> Option<usize> {
    let mut chars = text.chars();
    let mut gaps = 0;
    for wanted in query.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(actual) if actual == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(gaps)
}

/// 命令面板的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct CommandPaletteState {
    /// 面板是否可见。
    pub open: bool,
    /// 当前查询文本。
    pub query: String,
    /// 当前高亮结果的索引。
    pub selected: usize,
    /// 面板已知的所有命令。
    pub commands: Vec<Command>,
    /// 使用计数；数值越大表示越近期使用。
    recency: FxHashMap<u64, u32>,
    /// 单调递增的使用计数器。
    tick: u32,
}

impl CommandPaletteState {
    /// 基于给定命令列表创建关闭状态的面板。
    #[must_use]
    pub fn new(commands: Vec<Command>) -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            commands,
            recency: FxHashMap::default(),
            tick: 0,
        }
    }

    /// 清空查询并显示面板。
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }

    /// 隐藏面板。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 替换查询并重置高亮索引。
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected = 0;
    }

    /// 匹配结果的索引列表，最佳匹配在前。
    #[must_use]
    pub fn results(&self) -> Vec<usize> {
        // 将单调递增的 tick 映射为相对于当前最新调用的 1..=9 档位，避免超过 9 次调用后全部饱和为 9。
        let max_tick = self.recency.values().copied().max().unwrap_or(0);
        let mut scored: Vec<(i32, u32, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let tick = self.recency.get(&command.id).copied().unwrap_or(0);
                let relative_recency = if tick == 0 || max_tick == 0 {
                    0
                } else {
                    9u32.saturating_sub(max_tick.saturating_sub(tick)).max(1)
                };
                score_command(&self.query, command, relative_recency)
                    .map(|score| (score, tick, index))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        scored.into_iter().map(|(_, _, index)| index).collect()
    }

    /// 当前高亮的命令（若有）。
    #[must_use]
    pub fn selected_command(&self) -> Option<&Command> {
        let results = self.results();
        results
            .get(self.selected)
            .and_then(|index| self.commands.get(*index))
    }

    /// 移动高亮并在结果列表首尾循环。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let count = self.results().len();
        if count == 0 {
            return false;
        }
        let next = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
        let changed = next != self.selected;
        self.selected = next;
        changed
    }

    /// 记录某条命令被调用：将其设为最近使用。
    pub fn record_use(&mut self, id: u64) {
        self.tick = self.tick.saturating_add(1);
        self.recency.insert(id, self.tick);
    }

    /// 获取命令的近期使用排名。
    #[must_use]
    pub fn recency(&self, id: u64) -> u32 {
        self.recency.get(&id).copied().unwrap_or(0)
    }

    /// 按顺序返回列表可容纳的结果。
    #[must_use]
    pub fn visible_results(&self, desc: &CommandPaletteDesc) -> Vec<usize> {
        let mut results = self.results();
        results.truncate(desc.max_rows);
        results
    }
}

/// 命令面板绘制使用的颜色槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPaletteTokens {
    /// 面板表面背景色。
    pub surface: Token,
    /// 面板边框色。
    pub border: Token,
    /// 搜索框背景色。
    pub input: Token,
    /// 闲置行背景色。
    pub row: Token,
    /// 高亮行背景色。
    pub row_selected: Token,
    /// 标题文字颜色。
    pub text: Token,
    /// 高亮行标题文字颜色。
    pub text_selected: Token,
    /// 分组与快捷键提示文字颜色。
    pub hint: Token,
    /// 键盘焦点环颜色。
    pub focus_ring: Token,
}

impl CommandPaletteTokens {
    /// 标准配色方案。
    #[must_use]
    pub fn standard() -> Self {
        Self {
            surface: Token::OVERLAY_BG,
            border: Token::OUTLINE,
            // 搜索框底色对齐 WinUI `ControlFillColorDefault`（AutoSuggestBox）。
            input: Token::CONTROL_FILL,
            row: Token::NONE,
            row_selected: Token::PRIMARY_SOFT,
            text: Token::TEXT_PRIMARY,
            text_selected: Token::PRIMARY,
            hint: Token::TEXT_SECONDARY,
            focus_ring: Token::FOCUS_RING,
        }
    }
}

/// 发射面板外壳：表面、边框、搜索框与聚焦强调条。
#[must_use]
pub fn emit_command_palette(
    desc: &CommandPaletteDesc,
    state: &CommandPaletteState,
    tokens: CommandPaletteTokens,
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(4);
    // 面板圆角 `OverlayCornerRadius = 8`，1px 描边对齐浮层边框。
    primitives.push(outlined(
        desc.rect,
        PALETTE_RADIUS,
        tokens.surface,
        tokens.border,
        1.0,
        24.0,
        desc.z,
    ));
    let input = desc.input_rect();
    // 搜索框圆角 `ControlCornerRadius = 4`，底色 `ControlFillColorDefault`。
    primitives.push(surface(input, INPUT_RADIUS, tokens.input, desc.z));
    // 描边 `ControlStrokeColorDefault`：1px（`TextControlBorderThemeThickness = 1`）。
    primitives.push(outlined(
        input,
        INPUT_RADIUS,
        Token::NONE,
        Token::CONTROL_STROKE,
        1.0,
        0.0,
        desc.z,
    ));
    // 聚焦态底部 2px 强调条（`TextControlBorderThemeThicknessFocused = 1,1,1,2`）。
    // 图元始终存在，用不透明度表达“面板打开即有焦点”，保证图元数恒定。
    primitives.push(focus_underline(
        input,
        INPUT_RADIUS,
        desc.z,
        if state.open { 1.0 } else { 0.0 },
    ));
    WidgetOutput {
        primitives,
        payloads: Vec::new(),
        hit: None,
        a11y: None,
        focusable: false,
        ..Default::default()
    }
}

/// 发射单条结果行。
#[must_use]
pub fn emit_command_row(
    desc: &CommandPaletteDesc,
    state: &CommandPaletteState,
    visible_index: usize,
    control: ControlState,
    label: &[GlyphQuad],
    shortcut: &[GlyphQuad],
    tokens: CommandPaletteTokens,
) -> WidgetOutput {
    let results = state.visible_results(desc);
    let Some(command_index) = results.get(visible_index).copied() else {
        return WidgetOutput::default();
    };
    let Some(command) = state.commands.get(command_index) else {
        return WidgetOutput::default();
    };
    let rect = desc.row_rect(visible_index);
    if rect.size.y <= 0.0 {
        return WidgetOutput::default();
    }
    let highlighted = visible_index == state.selected;
    let disabled = control.disabled;
    let mut primitives = Vec::with_capacity(3 + label.len() + shortcut.len());
    let mut payloads = Vec::with_capacity(label.len() + shortcut.len());

    let fill = if highlighted {
        tokens.row_selected
    } else if !disabled && control.phase == PointerPhase::Hovered {
        // 建议行悬停底色对齐 WinUI `ListViewItemBackgroundPointerOver`
        // （`SubtleFillColorSecondary`，即 HOVER 令牌）。
        Token::HOVER
    } else {
        tokens.row
    };
    // 行圆角 `ControlCornerRadius = 4`。
    primitives.push(outlined(
        rect,
        ROW_RADIUS,
        fill,
        Token::NONE,
        0.0,
        0.0,
        desc.z,
    ));
    // 高亮标记槽位始终存在，仅用不透明度切换，保证各状态图元数恒定。
    let mut marker = instance(
        Rect::new(rect.origin.x, rect.origin.y, ROW_MARKER_WIDTH, rect.size.y),
        [0.0; 4],
        tokens.text_selected,
        Token::NONE,
        0.0,
        0.0,
        PrimType::Line,
        SubElement::Content,
        desc.z,
    );
    marker.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        marker.depth_u16(),
        if highlighted { 1.0 } else { 0.0 },
    );
    primitives.push(marker);
    glyphs(
        label,
        if highlighted {
            tokens.text_selected
        } else {
            tokens.text
        },
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    glyphs(
        shortcut,
        tokens.hint,
        desc.z,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        rect.inflate(1.0),
        ROW_RADIUS + 1.0,
        tokens.focus_ring,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!disabled).then(|| {
        HitRegion::new(command.id, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [ROW_RADIUS; 4],
            })
            .with_cursor(CursorKind::Pointer)
    });
    let mut a11y = A11yNode::new(Role::Button, command.title.clone(), rect)
        .clickable()
        .disabled(disabled);
    a11y.selected = highlighted;
    a11y.description = command.group.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<Command> {
        vec![
            Command::new(1, "Open File", "File").with_shortcut("Ctrl+O"),
            Command::new(2, "Close File", "File"),
            Command::new(3, "Open Folder", "File").with_keywords(&["directory", "workspace"]),
            Command::new(4, "Toggle Theme", "View").with_shortcut("Ctrl+T"),
            Command::new(5, "Rebuild", "Build"),
        ]
    }

    fn state() -> CommandPaletteState {
        CommandPaletteState::new(commands())
    }

    #[test]
    fn an_empty_query_lists_every_command() {
        let state = state();
        assert_eq!(state.results().len(), 5);
    }

    #[test]
    fn a_prefix_outranks_a_substring() {
        let mut state = state();
        state.set_query("open");
        let results = state.results();
        assert_eq!(results[0], 0, "Open File starts with the query");
        assert_eq!(results[1], 2, "Open Folder starts with the query");
        assert!(!results.contains(&1), "Close File is only a substring...");
    }

    #[test]
    fn hidden_keywords_participate_in_matching() {
        let mut state = state();
        state.set_query("workspace");
        assert_eq!(state.results(), vec![2]);
    }

    #[test]
    fn a_subsequence_matches_with_gaps() {
        let mut state = state();
        state.set_query("rbld");
        assert_eq!(state.results(), vec![4]);
    }

    #[test]
    fn recency_breaks_ties_but_cannot_invent_a_match() {
        let mut state = state();
        state.set_query("open");
        let before = state.results();
        state.record_use(2);
        assert!(
            !state.results().contains(&1),
            "Close File still does not match"
        );
        assert_eq!(state.results(), before);
        state.set_query("file");
        state.record_use(2);
        let after = state.results();
        assert_eq!(after[0], 1, "the most recent tie wins");
    }

    #[test]
    fn selection_wraps_and_moves() {
        let mut state = state();
        assert!(state.move_selection(1));
        assert_eq!(state.selected, 1);
        assert!(state.move_selection(-1));
        assert_eq!(state.selected, 0);
        assert!(state.move_selection(-1));
        assert_eq!(state.selected, 4);
        state.set_query("zzz");
        assert!(!state.move_selection(1));
    }

    #[test]
    fn the_selected_command_follows_the_query() {
        let mut state = state();
        state.set_query("toggle");
        assert_eq!(state.selected_command().map(|command| command.id), Some(4));
    }

    #[test]
    fn opening_resets_the_query() {
        let mut state = state();
        state.set_query("open");
        state.selected = 1;
        state.close();
        state.open();
        assert!(state.open);
        assert!(state.query.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn only_max_rows_are_visible() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let state = state();
        assert_eq!(state.visible_results(&desc).len(), 5);
        let mut narrow = desc.clone();
        narrow.max_rows = 2;
        assert_eq!(state.visible_results(&narrow).len(), 2);
    }

    #[test]
    fn rows_stack_below_the_search_field() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        assert_eq!(desc.input_rect().size.y, COMMAND_INPUT);
        assert_eq!(desc.list_rect().origin.y, COMMAND_INPUT);
        assert_eq!(desc.row_rect(1).origin.y, COMMAND_INPUT + COMMAND_ROW);
    }

    #[test]
    fn the_highlighted_row_is_selected_and_pickable() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let mut state = state();
        state.open();
        state.set_query("open");
        let tokens = CommandPaletteTokens::standard();
        let output = emit_command_row(&desc, &state, 0, ControlState::default(), &[], &[], tokens);
        let a11y = output.a11y.expect("a row is reachable");
        assert!(a11y.selected);
        assert_eq!(a11y.role, Role::Button);
        let hit = output.hit.expect("a row is pickable");
        assert_eq!(hit.node, 1);
    }

    #[test]
    fn an_out_of_range_row_emits_nothing() {
        let desc = CommandPaletteDesc::new(Rect::new(0.0, 0.0, 600.0, 400.0));
        let state = state();
        let output = emit_command_row(
            &desc,
            &state,
            99,
            ControlState::default(),
            &[],
            &[],
            CommandPaletteTokens::standard(),
        );
        assert!(output.primitives.is_empty());
    }

    #[test]
    fn recency_continues_to_distinguish_commands_beyond_nine_invocations() {
        let mut state = state();
        for _ in 0..12 {
            state.record_use(1);
        }
        state.record_use(3);
        state.set_query("open");
        assert_eq!(
            state.results()[0],
            2,
            "the most recently used command must still outrank older commands after >9 ticks"
        );
    }
}
