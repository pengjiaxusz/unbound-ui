//! 标签输入框：标签芯片（chip）、末尾编辑器与逗号/回车提交。
//!
//! 标签输入框在一个框内包含两个控件：已提交的芯片与实时编辑器。
//! 状态机将两者分离，因此删除芯片绝不会触及文本缓冲区，
//! 而输入 `,`（或粘贴 `"a, b"`）会在单次归约中提交所有分段，而非在缓冲区留下逗号。
//!
//!
//! 布局由调用方测量：控件无法塑形文本，因此 [`layout_tags`] 接受一个 `measure` 闭包。
//! 这既能保证针对实际字体精确换行，也能在单元测试中使用确定性桩函数测试几何布局。
//!
//!

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion, Shape};
use unbound_math::Rect;
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::text_input::{reduce_text_input, TextEditState, TextInputDesc, TextInputEvent};
use crate::{ControlState, WidgetOutput};

/// 单个标签芯片的高度。
pub const TAG_HEIGHT: f32 = 24.0;
/// 芯片之间的间距。
pub const TAG_GAP: f32 = 6.0;
/// 芯片内部的水平内边距。
pub const TAG_PADDING: f32 = 8.0;
/// 为芯片关闭按钮预留的宽度。
pub const TAG_CLOSE: f32 = 14.0;
/// 边框与芯片之间的内边距。
pub const TAG_INSET: f32 = 8.0;
/// 输入框与芯片的圆角半径。
pub const TAG_RADIUS: f32 = 8.0;

/// 标签输入框的纯数据描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct TagInputDesc {
    /// 输入框矩形。
    pub rect: Rect,
    /// 无障碍标签。
    pub label: String,
    /// 无芯片且无文本时显示的占位符。
    pub placeholder: String,
    /// 最大芯片数量。
    pub max_tags: usize,
    /// 单个芯片文本的最大字节长度。
    pub max_tag_bytes: usize,
    /// 是否禁用交互。
    pub disabled: bool,
    /// 绘制顺序键。
    pub z: u16,
}

impl TagInputDesc {
    /// 创建空的标签输入框描述符。
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            label: String::new(),
            placeholder: String::new(),
            max_tags: usize::MAX,
            max_tag_bytes: 64,
            disabled: false,
            z: 738,
        }
    }
}

/// 标签输入框消费的按键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagKey {
    /// 提交编辑器内容。
    Enter,
    /// 编辑器为空时删除最后一个芯片。
    Backspace,
    /// 选中上一个芯片。
    Previous,
    /// 选中下一个芯片。
    Next,
}

/// 单次归约的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagInputOutcome {
    /// 芯片或编辑器是否发生变化。
    pub changed: bool,
    /// 本次归约新提交的芯片列表。
    pub committed: Vec<String>,
}

/// 标签输入框的实时状态。
#[derive(Clone, Debug, PartialEq)]
pub struct TagInputState {
    tags: Vec<String>,
    input: TextEditState,
    selected: Option<usize>,
}

impl TagInputState {
    /// 创建空状态。
    #[must_use]
    pub fn new() -> Self {
        Self {
            tags: Vec::new(),
            input: TextEditState::new(),
            selected: None,
        }
    }

    /// 创建预填 `tags` 的状态。
    #[must_use]
    pub fn with_tags(tags: impl IntoIterator<Item = String>) -> Self {
        Self {
            tags: tags.into_iter().collect(),
            input: TextEditState::new(),
            selected: None,
        }
    }

    /// 已提交的芯片列表。
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// 末尾编辑器引用。
    #[must_use]
    pub fn input(&self) -> &TextEditState {
        &self.input
    }

    /// 当前选中的芯片索引。
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 将末尾编辑器文本设为 `text`。
    pub fn set_input(&mut self, text: impl Into<String>) {
        self.input = TextEditState::with_text(text);
    }

    /// 添加一个芯片。
    ///
    /// 自动裁剪首尾空白。空标签、重复标签、超长标签以及超出 `max_tags` 的芯片均会被拒绝。
    pub fn insert_tag(&mut self, text: &str, desc: &TagInputDesc) -> bool {
        let tag = text.trim();
        if tag.is_empty()
            || tag.len() > desc.max_tag_bytes
            || self.tags.iter().any(|existing| existing == tag)
            || self.tags.len() >= desc.max_tags
        {
            return false;
        }
        self.tags.push(tag.to_owned());
        true
    }

    /// 将编辑器内容提交为一个芯片。
    pub fn commit_input(&mut self, desc: &TagInputDesc) -> bool {
        let text = self.input.text().to_owned();
        if !self.insert_tag(&text, desc) {
            return false;
        }
        self.input = TextEditState::new();
        true
    }

    /// 移除指定芯片。
    pub fn delete_tag(&mut self, index: usize) -> bool {
        if index >= self.tags.len() {
            return false;
        }
        self.tags.remove(index);
        self.selected = match self.selected {
            Some(selected) if selected > index => Some(selected - 1),
            Some(selected) if selected == index => {
                if self.tags.is_empty() {
                    None
                } else {
                    Some(index.min(self.tags.len() - 1))
                }
            }
            other => other,
        };
        true
    }

    /// 按整块芯片移动选区，到达两端时夹紧。
    pub fn select_step(&mut self, delta: isize) -> bool {
        if self.tags.is_empty() {
            return false;
        }
        let current = self.selected.map_or(0, |index| index as isize);
        let next = (current + delta).clamp(0, self.tags.len() as isize - 1);
        let changed = self.selected != Some(next as usize);
        self.selected = Some(next as usize);
        changed
    }

    /// 路由文本事件，在分隔符或回车时提交。
    pub fn on_text_input(
        &mut self,
        desc: &TagInputDesc,
        event: &TextInputEvent,
    ) -> TagInputOutcome {
        if desc.disabled {
            return TagInputOutcome::default();
        }
        let mut editor = TextInputDesc::new(desc.rect);
        editor.label = desc.label.clone();
        editor.max_bytes = desc.max_tag_bytes;
        let outcome = reduce_text_input(&editor, &mut self.input, event);
        let mut committed = Vec::new();
        let mut changed = outcome.changed;

        if let TextInputEvent::Insert(text) = event {
            if text.contains(',') || text.contains('\n') {
                let buffer = self.input.text().to_owned();
                for segment in buffer.split([',', '\n']) {
                    if self.insert_tag(segment, desc) {
                        committed.push(segment.trim().to_owned());
                        changed = true;
                    }
                }
                self.input = TextEditState::new();
            }
        }
        TagInputOutcome { changed, committed }
    }

    /// 归约按键事件。
    pub fn key(&mut self, desc: &TagInputDesc, key: TagKey) -> TagInputOutcome {
        match key {
            TagKey::Enter => {
                let before = self.tags.len();
                let changed = self.commit_input(desc);
                let committed = if self.tags.len() > before {
                    self.tags.last().cloned().into_iter().collect()
                } else {
                    Vec::new()
                };
                TagInputOutcome { changed, committed }
            }
            TagKey::Backspace => {
                if self.input.text().is_empty() && self.tags.pop().is_some() {
                    self.selected = None;
                    return TagInputOutcome {
                        changed: true,
                        committed: Vec::new(),
                    };
                }
                TagInputOutcome::default()
            }
            TagKey::Previous => TagInputOutcome {
                changed: self.select_step(-1),
                committed: Vec::new(),
            },
            TagKey::Next => TagInputOutcome {
                changed: self.select_step(1),
                committed: Vec::new(),
            },
        }
    }
}

impl Default for TagInputState {
    fn default() -> Self {
        Self::new()
    }
}

/// 测量后的芯片几何信息。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagLayout {
    /// 按模型顺序排列的芯片矩形列表。
    pub chips: Vec<Rect>,
    /// 每个芯片的关闭按钮矩形。
    pub close: Vec<Rect>,
    /// 末尾编辑器矩形。
    pub input: Rect,
    /// 芯片共占用的行数。
    pub lines: usize,
}

/// 自动换行排布芯片并放置末尾编辑器。
///
/// `measure` 返回芯片标签的文本宽度；芯片总宽度为其加上内边距与关闭按钮宽度。
#[must_use]
pub fn layout_tags(
    desc: &TagInputDesc,
    state: &TagInputState,
    measure: impl Fn(&str) -> f32,
) -> TagLayout {
    let content = desc.rect.inflate(-TAG_INSET);
    let mut x = content.origin.x;
    let mut y = content.origin.y;
    let mut chips = Vec::with_capacity(state.tags.len());
    let mut close = Vec::with_capacity(state.tags.len());
    let mut lines = 1usize;
    let row_end = content.max().x;

    for tag in &state.tags {
        let width = (measure(tag) + TAG_PADDING * 2.0 + TAG_CLOSE).max(TAG_HEIGHT);
        if x > content.origin.x && x + width > row_end {
            x = content.origin.x;
            y += TAG_HEIGHT + TAG_GAP;
            lines += 1;
        }
        let chip = Rect::new(x, y, width, TAG_HEIGHT);
        chips.push(chip);
        close.push(Rect::new(
            chip.max().x - TAG_CLOSE,
            chip.origin.y,
            TAG_CLOSE,
            chip.size.y,
        ));
        x = chip.max().x + TAG_GAP;
    }

    let remaining = (row_end - x).max(80.0);
    let input = Rect::new(x, y, remaining, TAG_HEIGHT);
    TagLayout {
        chips,
        close,
        input,
        lines,
    }
}

/// 输出输入框外观、芯片与语义节点。
#[must_use]
pub fn emit_tag_input(
    node: u64,
    desc: &TagInputDesc,
    state: &TagInputState,
    control: ControlState,
    layout: &TagLayout,
    glyphs: &mut [GlyphQuad],
) -> WidgetOutput {
    let rect = desc.rect;
    let mut primitives = Vec::with_capacity(6 + layout.chips.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());

    let fill = if control.disabled {
        Token::CONTROL_DISABLED_BG
    } else if control.phase == crate::PointerPhase::Hovered {
        Token::CONTROL_HOVER
    } else {
        Token::INPUT_BG
    };
    primitives.push(surface(rect, TAG_RADIUS, fill, desc.z));
    primitives.push(outlined(
        rect,
        TAG_RADIUS,
        Token::NONE,
        if control.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if control.focused {
            Token::OUTLINE_STRONG
        } else {
            Token::OUTLINE
        },
        1.0,
        0.0,
        desc.z,
    ));

    for (index, chip) in layout.chips.iter().enumerate() {
        let selected = state.selected == Some(index);
        primitives.push(surface(
            *chip,
            TAG_HEIGHT * 0.5,
            Token::PRIMARY_SOFT,
            desc.z,
        ));
        if selected {
            primitives.push(outlined(
                *chip,
                TAG_HEIGHT * 0.5,
                Token::NONE,
                Token::PRIMARY,
                2.0,
                0.0,
                desc.z,
            ));
        }
        if let Some(close) = layout.close.get(index) {
            primitives.push(instance(
                Rect::new(close.center().x - 3.0, close.center().y - 0.75, 6.0, 1.5),
                [0.0; 4],
                Token::TEXT_SECONDARY,
                Token::NONE,
                0.0,
                0.0,
                PrimType::Line,
                SubElement::Content,
                desc.z,
            ));
        }
    }

    // 空编辑器光标保持固定槽位：聚焦仅改变其不透明度，使图元数量在各状态下保持不变。
    //
    let mut caret = surface(
        Rect::new(
            layout.input.origin.x,
            layout.input.origin.y + 4.0,
            1.5,
            TAG_HEIGHT - 8.0,
        ),
        0.0,
        Token::PRIMARY,
        desc.z,
    );
    caret.z_and_opacity = GpuPrimitiveInstance::pack_z_opacity(
        caret.depth_u16(),
        if state.input.text().is_empty() && control.focused && !control.disabled {
            1.0
        } else {
            0.0
        },
    );
    primitives.push(caret);

    let ink = if control.disabled {
        Token::TEXT_DISABLED
    } else if state.tags.is_empty() && state.input.text().is_empty() {
        Token::TEXT_SECONDARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_glyphs(glyphs, ink, desc.z, &mut primitives, &mut payloads);

    primitives.push(focus_ring(
        rect.inflate(3.0),
        TAG_RADIUS + 3.0,
        Token::FOCUS_RING,
        2.0,
        desc.z,
        control.focused,
    ));

    let hit = (!control.disabled).then(|| {
        HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), rect)
            .with_shape(Shape::Rounded {
                radii: [TAG_RADIUS; 4],
            })
            .with_cursor(CursorKind::Text)
    });
    let mut a11y = A11yNode::new(Role::Group, desc.label.clone(), rect)
        .clickable()
        .disabled(control.disabled);
    a11y.value = state.tags.join(", ");
    a11y.description = desc.placeholder.clone();

    WidgetOutput {
        primitives,
        payloads,
        hit,
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> TagInputDesc {
        TagInputDesc::new(Rect::new(10.0, 10.0, 300.0, 80.0))
    }

    #[test]
    fn chips_are_trimmed_and_deduplicated() {
        let desc = desc();
        let mut state = TagInputState::new();
        assert!(state.insert_tag("  rust ", &desc));
        assert_eq!(state.tags(), ["rust"]);
        assert!(!state.insert_tag("rust", &desc));
        assert!(!state.insert_tag("   ", &desc));
        assert_eq!(state.tags().len(), 1);
    }

    #[test]
    fn limits_are_enforced() {
        let mut desc = desc();
        desc.max_tags = 3;
        desc.max_tag_bytes = 4;
        let mut state = TagInputState::new();
        assert!(state.insert_tag("one", &desc));
        assert!(state.insert_tag("two", &desc));
        assert!(state.insert_tag("four", &desc));
        assert!(!state.insert_tag("five", &desc), "max tags");
        assert!(!state.insert_tag("long", &desc));
        assert_eq!(state.tags().len(), 3);
    }

    #[test]
    fn committing_clears_the_editor() {
        let desc = desc();
        let mut state = TagInputState::new();
        state.set_input("hello");
        assert!(state.commit_input(&desc));
        assert_eq!(state.tags(), ["hello"]);
        assert!(state.input().text().is_empty());
    }

    #[test]
    fn a_comma_commits_every_segment() {
        let desc = desc();
        let mut state = TagInputState::new();
        let outcome = state.on_text_input(&desc, &TextInputEvent::Insert("a, b,c".into()));
        assert_eq!(outcome.committed, ["a", "b", "c"]);
        assert_eq!(state.tags(), ["a", "b", "c"]);
        assert!(state.input().text().is_empty());
    }

    #[test]
    fn backspace_on_an_empty_editor_pops_the_last_chip() {
        let desc = desc();
        let mut state = TagInputState::with_tags(["a".to_owned(), "b".to_owned()]);
        let outcome = state.key(&desc, TagKey::Backspace);
        assert!(outcome.changed);
        assert_eq!(state.tags(), ["a"]);
        assert!(state.key(&desc, TagKey::Backspace).changed);
        assert!(!state.key(&desc, TagKey::Backspace).changed);
    }

    #[test]
    fn deleting_a_chip_keeps_the_selection_valid() {
        let mut state = TagInputState::with_tags(["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        state.select_step(1);
        state.select_step(1);
        assert_eq!(state.selected(), Some(2));
        assert!(state.delete_tag(1));
        assert_eq!(state.selected(), Some(1));
        assert!(!state.delete_tag(9));
    }

    #[test]
    fn chips_wrap_and_the_editor_takes_the_rest_of_the_row() {
        let desc = TagInputDesc::new(Rect::new(0.0, 0.0, 120.0, 100.0));
        let state =
            TagInputState::with_tags(["one".to_owned(), "two".to_owned(), "three".to_owned()]);
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        assert_eq!(layout.chips.len(), 3);
        assert!(layout.lines >= 2);
        assert!(layout.chips[2].origin.y > layout.chips[0].origin.y);
        assert!(layout.input.origin.x >= 0.0);
        assert!(layout.input.size.x >= 80.0);
    }

    #[test]
    fn the_emitter_reports_a_group_with_the_chip_values() {
        let desc = desc();
        let state = TagInputState::with_tags(["rust".to_owned(), "gpu".to_owned()]);
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        let output = emit_tag_input(11, &desc, &state, ControlState::default(), &layout, &mut []);
        let a11y = output.a11y.expect("reachable");
        assert_eq!(a11y.role, Role::Group);
        assert_eq!(a11y.value, "rust, gpu");
        assert_eq!(
            output.hit.as_ref().map(|hit| hit.cursor),
            Some(Some(CursorKind::Text))
        );
    }

    #[test]
    fn stable_shape_across_states() {
        let desc = desc();
        let state = TagInputState::new();
        let layout = layout_tags(&desc, &state, |tag| tag.len() as f32 * 8.0);
        let mut counts = Vec::new();
        for phase in [
            crate::PointerPhase::Idle,
            crate::PointerPhase::Hovered,
            crate::PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for disabled in [false, true] {
                    for selected in [false, true] {
                        let control = ControlState {
                            phase,
                            focused,
                            disabled,
                            selected,
                        };
                        let output = emit_tag_input(11, &desc, &state, control, &layout, &mut []);
                        counts.push(output.primitives.len());
                    }
                }
            }
        }
        assert!(
            counts.windows(2).all(|pair| pair[0] == pair[1]),
            "{counts:?}"
        );
    }
}
