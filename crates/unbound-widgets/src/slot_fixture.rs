//! WinUI 3 基准控件统一标准插桩辅助库与配方契约基础。
//!
//! 依据 ADR 0016 与 Spec #79，本模块提供容器与复合控件通用的标准插桩生成器：
//! 1. 通用路径/分段插桩辅助：固定输出 `Root > Folder > File.txt` 的标准文本与几何（用于 `BreadcrumbBar`、`SelectorBar` 等）；
//! 2. 通用树形展开插桩辅助：固定输出 2 级展开结构 `Root`（首项选中态）+ 子节点 `Child`（用于 `TreeView` 等）；
//! 3. 通用垂直列表/堆叠插桩辅助：固定输出统一字号与间距的 `Row 1`、`Row 2`、`Row 3`（用于 `StackPanel`、`ScrollView`、`ListView` 等）；
//! 4. 通用递归禁用态（`disabled`）传播机制：保证容器处于 `disabled` 状态时，所有子插桩文本、图标与指示器同步应用半透明度衰减与变灰。

use unbound_gpu::data::{GpuPrimitiveInstance, PrimType};
use unbound_math::{Rect, Vec2};
use unbound_theme::Token;

use crate::ergonomics::TextCtx;
use crate::primitive;
use crate::{AlignX, AlignY, WidgetOutput};

/// 标准路径插桩文本项（固定 3 级）。
pub const CANONICAL_PATH_ITEMS: [&str; 3] = ["Root", "Folder", "File.txt"];

/// 标准路径分隔符文本。
pub const CANONICAL_PATH_SEPARATOR: &str = ">";

/// 标准分段插桩文本项（固定 3 段）。
pub const CANONICAL_SEGMENTED_ITEMS: [&str; 3] = ["Day", "Week", "Month"];

/// 标准树形根节点标签。
pub const CANONICAL_TREE_ROOT: &str = "Root";

/// 标准树形展开子节点标签。
pub const CANONICAL_TREE_CHILD: &str = "Child";

/// 标准垂直堆叠纯文本行项（固定 3 行）。
pub const CANONICAL_ROW_ITEMS: [&str; 3] = ["Row 1", "Row 2", "Row 3"];

/// 标准可交互列表条目项（固定 3 项）。
pub const CANONICAL_LIST_ITEMS: [&str; 3] = ["Item 1", "Item 2", "Item 3"];

/// 标准网格条目项（固定 2×2，共 4 项）。
pub const CANONICAL_GRID_ITEMS: [&str; 4] = ["Item 1", "Item 2", "Item 3", "Item 4"];

/// 标准翻页视图页签项（固定 2 页）。
pub const CANONICAL_FLIP_PAGES: [&str; 2] = ["Page 1", "Page 2"];

/// 标准下拉刷新提示文本。
pub const CANONICAL_REFRESH_TEXT: &str = "Pull to refresh";

/// 标准插桩文本字号（WinUI 3 默认 TextBlock 字号）。
pub const STANDARD_FONT_SIZE: f32 = 14.0;

/// 标准树形行高（WinUI 3 TreeView 标准行高）。
pub const STANDARD_TREE_ITEM_HEIGHT: f32 = 36.0;

/// 标准列表项行高（WinUI 3 ListView 标准行高）。
pub const STANDARD_LIST_ITEM_HEIGHT: f32 = 40.0;

/// 标准堆叠项垂直间距（WinUI 3 StackPanel 常用间距）。
pub const STANDARD_STACK_SPACING: f32 = 8.0;

/// 标准树形层级缩进量。
pub const STANDARD_TREE_INDENT: f32 = 16.0;

/// 标准网格视图固有宽度。
pub const STANDARD_GRID_WIDTH: f32 = 260.0;

/// 标准网格视图固有高度。
pub const STANDARD_GRID_HEIGHT: f32 = 140.0;

/// 标准网格单元圆角半径。
pub const STANDARD_GRID_CELL_RADIUS: f32 = 6.0;

/// 标准翻页视图固有宽度。
pub const STANDARD_FLIP_WIDTH: f32 = 220.0;

/// 标准翻页视图固有高度。
pub const STANDARD_FLIP_HEIGHT: f32 = 120.0;

/// 标准下拉刷新容器固有宽度。
pub const STANDARD_REFRESH_WIDTH: f32 = 260.0;

/// 标准下拉刷新容器固有高度。
pub const STANDARD_REFRESH_HEIGHT: f32 = 160.0;

/// 标准环形刷新指示器直径。
pub const STANDARD_REFRESH_INDICATOR_SIZE: f32 = 24.0;

/// 标准标注滚动条固有宽度。
pub const STANDARD_SCROLL_BAR_WIDTH: f32 = 16.0;

/// 标准标注滚动条固有高度。
pub const STANDARD_SCROLL_BAR_HEIGHT: f32 = 160.0;

/// 标准标注滚动条轨道宽度。
pub const STANDARD_SCROLL_BAR_TRACK_WIDTH: f32 = 8.0;

/// 标准选中指示胶囊条宽度。
pub const STANDARD_PILL_WIDTH: f32 = 3.0;

/// 标准选中指示胶囊条高度。
pub const STANDARD_PILL_HEIGHT: f32 = 16.0;

/// 标准选中指示胶囊条圆角半径。
pub const STANDARD_PILL_RADIUS: f32 = 1.5;

/// WinUI 3 官方禁用态不透明度衰减系数（36%）。
pub const WINUI_DISABLED_OPACITY: f32 = 0.36;

/// 插桩状态配置契约。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FixtureState {
    /// 是否处于禁用状态。
    pub disabled: bool,
    /// 是否处于键盘聚焦状态。
    pub focused: bool,
    /// 是否处于选中/勾选状态。
    pub checked: bool,
    /// 交互项初始选中索引（默认为 0）。
    pub selected_index: usize,
}

impl FixtureState {
    /// 默认常规态。
    #[must_use]
    pub const fn normal() -> Self {
        Self {
            disabled: false,
            focused: false,
            checked: false,
            selected_index: 0,
        }
    }

    /// 禁用态。
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            disabled: true,
            focused: false,
            checked: false,
            selected_index: 0,
        }
    }

    /// 选中/勾选态。
    #[must_use]
    pub const fn checked() -> Self {
        Self {
            disabled: false,
            focused: false,
            checked: true,
            selected_index: 0,
        }
    }

    /// 聚焦态。
    #[must_use]
    pub const fn focused() -> Self {
        Self {
            disabled: false,
            focused: true,
            checked: false,
            selected_index: 0,
        }
    }

    /// 设置禁用状态。
    #[must_use]
    pub const fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置选中索引。
    #[must_use]
    pub const fn with_selected(mut self, selected_index: usize) -> Self {
        self.selected_index = selected_index;
        self
    }
}

/// 通用路径/分段插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct PathFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各路径项对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 各分隔符对应的包围盒矩形。
    pub separator_rects: Vec<Rect>,
    /// 插桩内容测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用树形展开插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TreeFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 根节点包围盒矩形。
    pub root_rect: Rect,
    /// 子节点包围盒矩形。
    pub child_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用垂直纯文本行插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct TextRowsFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各文本行对应的包围盒矩形。
    pub row_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用可交互列表行插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct ListRowsFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各行条目对应的包围盒矩形。
    pub item_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用网格集合视图插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct GridFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 各单元格对应的包围盒矩形。
    pub cell_rects: Vec<Rect>,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用翻页视图插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct FlipViewFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 内容卡片包围盒矩形。
    pub card_rect: Rect,
    /// 上一页导航按钮矩形。
    pub prev_button_rect: Rect,
    /// 下一页导航按钮矩形。
    pub next_button_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用下拉刷新容器插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct RefreshFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 顶部环形指示器外框矩形。
    pub indicator_rect: Rect,
    /// 下方内容卡片矩形。
    pub card_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 通用标注滚动条插桩输出与几何排版信息。
#[derive(Clone, Debug, Default)]
pub struct AnnotatedScrollBarFixtureOutput {
    /// 图元与载荷输出。
    pub output: WidgetOutput,
    /// 滚动条轨道矩形。
    pub track_rect: Rect,
    /// 滑块矩形。
    pub thumb_rect: Rect,
    /// 测得的整体尺寸。
    pub total_size: Vec2,
}

/// 发射标准化 3 级路径插桩（`Root > Folder > File.txt`）。
///
/// 几何与排版规则：
/// - 水平顺序排布 `Root`、`>`、`Folder`、`>`、`File.txt`；
/// - 标签项与分隔符之间固定留有 4px 水平间距；
/// - 末尾项为当前项：未禁用时字重为 600、颜色为 [`Token::TEXT_PRIMARY`]；
/// - 前序项与分隔符：未禁用时字重为 400、颜色为 [`Token::TEXT_SECONDARY`]；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，所有文本与分隔符强制降级为字重 400 与 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_path_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> PathFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut item_rects = Vec::with_capacity(3);
    let mut separator_rects = Vec::with_capacity(2);

    let row_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        32.0
    };
    let gap = 4.0;
    let mut cur_x = rect.origin.x;

    let sep_measure = text_ctx.measure_line(CANONICAL_PATH_SEPARATOR, 12.0, 400);
    let sep_w = sep_measure.x.ceil().max(8.0);

    for (idx, &item_text) in CANONICAL_PATH_ITEMS.iter().enumerate() {
        let is_last = idx == CANONICAL_PATH_ITEMS.len() - 1;
        let weight = if is_last && !state.disabled { 600 } else { 400 };
        let text_color = if state.disabled {
            Token::TEXT_DISABLED
        } else if is_last {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        };

        let item_measure = text_ctx.measure_line(item_text, STANDARD_FONT_SIZE, weight);
        let item_w = item_measure.x.ceil().max(20.0);
        let item_r = Rect::new(cur_x, rect.origin.y, item_w, row_h);
        item_rects.push(item_r);

        let quads = text_ctx.shape_line_styled(
            item_text,
            item_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_color,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_x += item_w;

        if !is_last {
            cur_x += gap;
            let sep_r = Rect::new(cur_x, rect.origin.y, sep_w, row_h);
            separator_rects.push(sep_r);

            let sep_color = if state.disabled {
                Token::TEXT_DISABLED
            } else {
                Token::TEXT_SECONDARY
            };

            let sep_quads = text_ctx.shape_line_styled(
                CANONICAL_PATH_SEPARATOR,
                sep_r,
                12.0,
                400,
                None,
                AlignX::Center,
                AlignY::Middle,
            );
            primitive::glyphs(
                &sep_quads,
                sep_color,
                z + 2,
                &mut out.primitives,
                &mut out.payloads,
            );

            cur_x += sep_w + gap;
        }
    }

    let total_w = cur_x - rect.origin.x;
    PathFixtureOutput {
        output: out,
        item_rects,
        separator_rects,
        total_size: Vec2::new(total_w, row_h),
    }
}

/// 发射标准化分段选择条插桩（`Day | Week | Month`）。
///
/// 几何与排版规则：
/// - 将 `rect` 水平等分为 3 个槽位；
/// - 选中项（由 `state.selected_index` 决定，默认 0）：
///   - 未禁用时：绘制 [`Token::HOVER`] 背景表面与左侧 3×16px 胶囊指示条（[`Token::PRIMARY`]），文本字重 600、颜色 [`Token::TEXT_PRIMARY`]；
///   - 禁用时：抑制背景表面与胶囊条，文本字重 400、颜色 [`Token::TEXT_DISABLED`]；
/// - 未选中项：
///   - 未禁用时：字重 400、颜色 [`Token::TEXT_SECONDARY`]；
///   - 禁用时：字重 400、颜色 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_segmented_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> WidgetOutput {
    let mut out = WidgetOutput::default();
    let slot_count = CANONICAL_SEGMENTED_ITEMS.len() as f32;
    let pad = 2.0;
    let available_w = (rect.width() - pad * 2.0).max(1.0);
    let slot_w = available_w / slot_count;
    let slot_h = (rect.height() - pad * 2.0).max(1.0);

    for (idx, &text) in CANONICAL_SEGMENTED_ITEMS.iter().enumerate() {
        let slot_x = rect.origin.x + pad + idx as f32 * slot_w;
        let slot_r = Rect::new(slot_x, rect.origin.y + pad, slot_w, slot_h);
        let is_selected = idx == state.selected_index;

        if is_selected && !state.disabled {
            out.primitives
                .push(primitive::surface(slot_r, 4.0, Token::HOVER, z + 2));

            let pill = Rect::new(
                slot_r.origin.x + 3.0,
                slot_r.origin.y + (slot_h - STANDARD_PILL_HEIGHT) * 0.5,
                STANDARD_PILL_WIDTH,
                STANDARD_PILL_HEIGHT,
            );
            out.primitives.push(primitive::surface(
                pill,
                STANDARD_PILL_RADIUS,
                Token::PRIMARY,
                z + 5,
            ));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    slot_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        let text_color = if state.disabled {
            Token::TEXT_DISABLED
        } else if is_selected {
            Token::TEXT_PRIMARY
        } else {
            Token::TEXT_SECONDARY
        };

        let weight = if is_selected && !state.disabled {
            600
        } else {
            400
        };

        let quads = text_ctx.shape_line_styled(
            text,
            slot_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_color,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    out
}

/// 发射标准化 2 级展开树形插桩（根节点 `Root` 选中展开 + 子节点 `Child` 缩进）。
///
/// 几何与排版规则：
/// - 根节点（深度 0，行高 36px）：
///   - 包含展开箭头（朝下 chevron，thickness 1.5px）；
///   - 首项处于选中态：左侧垂直居中绘制 3×16px、圆角 1.5px 的 Accent 胶囊条（[`Token::PRIMARY`]），填充 [`Token::HOVER`] 背景；
///   - 标签 `"Root"` 位于箭头右侧（x 偏移 32px）；
/// - 子节点（深度 1，缩进 16px，行高 36px）：
///   - 绘制垂直深度连线（1.0px 宽度，[`Token::DIVIDER`]）；
///   - 标签 `"Child"` 位于连线右侧（x 偏移 16px）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，抑制 Accent 胶囊条与选中背景，chevron 指示器、引导线与文本全量变灰衰减为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_tree_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TreeFixtureOutput {
    let mut out = WidgetOutput::default();
    let item_h = STANDARD_TREE_ITEM_HEIGHT;
    let tree_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        220.0
    };

    // 1. Root 根节点行（深度 0，首项选中）
    let root_y = rect.origin.y + 4.0;
    let root_r = Rect::new(
        rect.origin.x + 4.0,
        root_y,
        (tree_w - 8.0).max(1.0),
        item_h - 4.0,
    );
    let root_selected = !state.disabled && (state.checked || state.selected_index == 0);

    if root_selected {
        out.primitives
            .push(primitive::surface(root_r, 4.0, Token::HOVER, z + 2));

        let pill = Rect::new(
            root_r.origin.x,
            root_y + (item_h - 4.0 - STANDARD_PILL_HEIGHT) * 0.5,
            STANDARD_PILL_WIDTH,
            STANDARD_PILL_HEIGHT,
        );
        out.primitives.push(primitive::surface(
            pill,
            STANDARD_PILL_RADIUS,
            Token::PRIMARY,
            z + 5,
        ));

        if state.focused {
            out.primitives.push(primitive::focus_ring(
                root_r.inflate(1.0),
                5.0,
                Token::FOCUS_RING,
                2.0,
                z + 10,
                true,
            ));
        }
    }

    // 根节点折叠/展开指示箭头（已展开：朝下箭头）
    let chevron_cx = root_r.origin.x + 16.0;
    let chevron_cy = root_y + (item_h - 4.0) * 0.5;
    let chevron_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_SECONDARY
    };
    primitive::curve_stroke(
        [chevron_cx - 4.0, chevron_cy - 2.0],
        [chevron_cx, chevron_cy + 2.0],
        1.5,
        chevron_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );
    primitive::curve_stroke(
        [chevron_cx, chevron_cy + 2.0],
        [chevron_cx + 4.0, chevron_cy - 2.0],
        1.5,
        chevron_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 2. Child 子节点行（深度 1，缩进 16px）
    let indent = STANDARD_TREE_INDENT;
    let child_y = root_y + item_h;
    let child_r = Rect::new(
        rect.origin.x + 4.0 + indent,
        child_y,
        (tree_w - 8.0 - indent).max(1.0),
        item_h - 4.0,
    );

    // 子节点深度的引导连线
    let guide_token = if state.disabled {
        Token::CONTROL_DISABLED_BORDER
    } else {
        Token::DIVIDER
    };
    out.primitives.push(primitive::surface(
        Rect::new(root_r.origin.x + 16.0, child_y, 1.0, item_h - 4.0),
        0.0,
        guide_token,
        z + 3,
    ));

    // 文本排版
    let text_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let root_weight = if root_selected { 600 } else { 400 };
    let root_label_r = Rect::new(
        root_r.origin.x + 32.0,
        root_r.origin.y,
        (root_r.max().x - (root_r.origin.x + 32.0)).max(1.0),
        root_r.height(),
    );
    let root_quads = text_ctx.shape_line_styled(
        CANONICAL_TREE_ROOT,
        root_label_r,
        STANDARD_FONT_SIZE,
        root_weight,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &root_quads,
        text_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    let child_label_r = Rect::new(
        child_r.origin.x + 16.0,
        child_r.origin.y,
        (child_r.max().x - (child_r.origin.x + 16.0)).max(1.0),
        child_r.height(),
    );
    let child_quads = text_ctx.shape_line_styled(
        CANONICAL_TREE_CHILD,
        child_label_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(
        &child_quads,
        text_ink,
        z + 4,
        &mut out.primitives,
        &mut out.payloads,
    );

    TreeFixtureOutput {
        output: out,
        root_rect: root_r,
        child_rect: child_r,
        total_size: Vec2::new(tree_w, item_h * 2.0 + 8.0),
    }
}

/// 发射标准化垂直纯文本列表插桩（`Row 1`、`Row 2`、`Row 3`），用于 `StackPanel`、`ScrollView` 等。
///
/// 几何与排版规则：
/// - 固定输出 3 行纯文本，字号 14px，间距由 `spacing` 控制（默认建议 8px）；
/// - 未禁用状态：使用 [`Token::TEXT_PRIMARY`]；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，所有行文本同步应用 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_text_rows_fixture(
    rect: Rect,
    spacing: f32,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> TextRowsFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut row_rects = Vec::with_capacity(3);

    let line_h = 20.0;
    let mut cur_y = rect.origin.y;
    let text_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };

    let row_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        200.0
    };

    for &row_text in &CANONICAL_ROW_ITEMS {
        let row_r = Rect::new(rect.origin.x, cur_y, row_w, line_h);
        row_rects.push(row_r);

        let quads = text_ctx.shape_line_styled(
            row_text,
            row_r,
            STANDARD_FONT_SIZE,
            400,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 2,
            &mut out.primitives,
            &mut out.payloads,
        );

        cur_y += line_h + spacing;
    }

    let total_h = (cur_y - spacing) - rect.origin.y;
    TextRowsFixtureOutput {
        output: out,
        row_rects,
        total_size: Vec2::new(row_w, total_h),
    }
}

/// 发射标准化可交互垂直列表行插桩，用于 `ListView`、`ListBox` 等。
///
/// 几何与排版规则：
/// - 固定输出 3 个标准行（标准行高 40px）；
/// - 首项处于选中态（`state.selected_index == 0`）：
///   - 未禁用时：绘制 [`Token::HOVER`] 背景表面与左侧 3×16px 胶囊指示条（[`Token::PRIMARY`]），文本字重 500、颜色 [`Token::TEXT_PRIMARY`]；
///   - 禁用时：抑制背景表面与胶囊指示条，文本字重 400、颜色 [`Token::TEXT_DISABLED`]；
/// - 其他行：文本字重 400，未禁用时为 [`Token::TEXT_PRIMARY`]，禁用时为 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_list_rows_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> ListRowsFixtureOutput {
    let mut out = WidgetOutput::default();
    let mut item_rects = Vec::with_capacity(3);

    let item_h = STANDARD_LIST_ITEM_HEIGHT;
    let list_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        220.0
    };

    for (idx, &text) in CANONICAL_LIST_ITEMS.iter().enumerate() {
        let y = rect.origin.y + idx as f32 * item_h;
        let row_r = Rect::new(
            rect.origin.x + 3.0,
            y + 2.0,
            (list_w - 6.0).max(1.0),
            item_h - 4.0,
        );
        item_rects.push(row_r);

        let is_selected = !state.disabled && (idx == state.selected_index);

        if is_selected {
            out.primitives
                .push(primitive::surface(row_r, 4.0, Token::HOVER, z + 2));

            let pill = Rect::new(
                row_r.origin.x,
                y + (item_h - STANDARD_PILL_HEIGHT) * 0.5,
                STANDARD_PILL_WIDTH,
                STANDARD_PILL_HEIGHT,
            );
            out.primitives.push(primitive::surface(
                pill,
                STANDARD_PILL_RADIUS,
                Token::PRIMARY,
                z + 5,
            ));

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    row_r.inflate(1.0),
                    5.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 10,
                    true,
                ));
            }
        }

        let text_ink = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };

        let label_r = Rect::new(
            rect.origin.x + 16.0,
            row_r.origin.y,
            (row_r.max().x - (rect.origin.x + 16.0)).max(1.0),
            row_r.height(),
        );

        let weight = if is_selected { 500 } else { 400 };
        let quads = text_ctx.shape_line_styled(
            text,
            label_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Left,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 4,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    ListRowsFixtureOutput {
        output: out,
        item_rects,
        total_size: Vec2::new(list_w, item_h * 3.0),
    }
}

/// 发射标准化 2D 网格单元插桩（`Item 1` 到 `Item 4`，2×2 网格）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 260px、高 140px；
/// - 外围绘制圆角 8px 的控件背景卡片；
/// - 内部等分为 2 列 × 2 行单元格，单元内边距 6px、单元间距 6px，单元圆角 6px；
/// - 首项处于选中态（`state.selected_index == 0`）：
///   - 未禁用时：绘制 [`Token::PRIMARY_SOFT`] 填充、[`Token::PRIMARY`] 2.0px 高亮边框与右上角勾选徽章；
///   - 禁用时：降级为 [`Token::CONTROL_DISABLED_BG`] 填充与 [`Token::CONTROL_DISABLED_BORDER`] 边框，抑制高亮边框与徽章；
/// - 其他未选中项：
///   - 未禁用时：绘制 [`Token::SURFACE_SUNKEN`] 填充与 [`Token::CONTROL_STROKE`] 1.0px 边框；
///   - 禁用时：绘制 [`Token::CONTROL_DISABLED_BG`] 填充与 [`Token::CONTROL_DISABLED_BORDER`] 边框；
/// - 单元文本：字号 14px，居中排版。选中项字重 600、未禁用时为 [`Token::TEXT_PRIMARY`]；禁用时统一为字重 400 与 [`Token::TEXT_DISABLED`]。
pub fn emit_canonical_grid_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> GridFixtureOutput {
    let mut out = WidgetOutput::default();
    let grid_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_GRID_WIDTH
    };
    let grid_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_GRID_HEIGHT
    };
    let grid_rect = Rect::new(rect.origin.x, rect.origin.y, grid_w, grid_h);

    out.primitives.push(primitive::outlined(
        grid_rect,
        8.0,
        if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        z,
    ));

    let pad = 6.0;
    let gap = 6.0;
    let cols = 2;
    let rows = 2;
    let cell_w = ((grid_w - pad * 2.0 - gap * (cols - 1) as f32) / cols as f32).max(1.0);
    let cell_h = ((grid_h - pad * 2.0 - gap * (rows - 1) as f32) / rows as f32).max(1.0);

    let mut cell_rects = Vec::with_capacity(4);

    for (idx, &text) in CANONICAL_GRID_ITEMS.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let x = grid_rect.origin.x + pad + col as f32 * (cell_w + gap);
        let y = grid_rect.origin.y + pad + row as f32 * (cell_h + gap);
        let cell_r = Rect::new(x, y, cell_w, cell_h);
        cell_rects.push(cell_r);

        let is_selected = !state.disabled && (idx == state.selected_index);

        let fill = if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else if is_selected {
            Token::PRIMARY_SOFT
        } else {
            Token::SURFACE_SUNKEN
        };

        let border = if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else if is_selected {
            Token::PRIMARY
        } else {
            Token::CONTROL_STROKE
        };

        out.primitives.push(primitive::outlined(
            cell_r,
            STANDARD_GRID_CELL_RADIUS,
            fill,
            border,
            if is_selected { 2.0 } else { 1.0 },
            0.0,
            z + 2,
        ));

        if is_selected {
            let badge_size = 18.0;
            let badge_x = cell_r.max().x - badge_size - 6.0;
            let badge_y = cell_r.origin.y + 6.0;
            let badge_r = Rect::new(badge_x, badge_y, badge_size, badge_size);

            out.primitives.push(primitive::surface(
                badge_r,
                badge_size * 0.5,
                Token::PRIMARY,
                z + 4,
            ));

            let mark_start = [badge_x + 4.5, badge_y + 9.0];
            let mark_elbow = [badge_x + 7.5, badge_y + 12.0];
            let mark_tip = [badge_x + 13.5, badge_y + 5.5];
            primitive::curve_stroke(
                mark_start,
                mark_elbow,
                1.5,
                Token::ON_PRIMARY,
                z + 5,
                &mut out.primitives,
                &mut out.payloads,
            );
            primitive::curve_stroke(
                mark_elbow,
                mark_tip,
                1.5,
                Token::ON_PRIMARY,
                z + 5,
                &mut out.primitives,
                &mut out.payloads,
            );

            if state.focused {
                out.primitives.push(primitive::focus_ring(
                    cell_r.inflate(1.0),
                    7.0,
                    Token::FOCUS_RING,
                    2.0,
                    z + 8,
                    true,
                ));
            }
        }

        let text_ink = if state.disabled {
            Token::TEXT_DISABLED
        } else {
            Token::TEXT_PRIMARY
        };

        let weight = if is_selected { 600 } else { 400 };
        let quads = text_ctx.shape_line_styled(
            text,
            cell_r,
            STANDARD_FONT_SIZE,
            weight,
            None,
            AlignX::Center,
            AlignY::Middle,
        );
        primitive::glyphs(
            &quads,
            text_ink,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    GridFixtureOutput {
        output: out,
        cell_rects,
        total_size: Vec2::new(grid_w, grid_h),
    }
}

/// 发射标准化翻页视图插桩（内容卡片 `Page 1` 与翻页导航前后按钮）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 220px、高 120px；
/// - 外框为圆角 4px 的内容卡片背景（[`Token::CARD_BG`]、边框 [`Token::CARD_BORDER`]）；
/// - 中央水平与垂直居中排版当前页文本 `"Page 1"`（未禁用时字重 400、颜色 [`Token::TEXT_PRIMARY`]）；
/// - 左侧垂直居中放置上一页导航按钮（28×28px，向左 chevron 折线，因处于首页故呈次级半透明或禁用态）；
/// - 右侧垂直居中放置下一页导航按钮（28×28px，向右 chevron 折线，未禁用时为活跃色 [`Token::TEXT_PRIMARY`]）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，卡片背景、边框、页文本与全部导航箭头均降级为变灰与衰减令牌。
pub fn emit_canonical_flip_view_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> FlipViewFixtureOutput {
    let mut out = WidgetOutput::default();
    let flip_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_FLIP_WIDTH
    };
    let flip_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_FLIP_HEIGHT
    };
    let card_r = Rect::new(rect.origin.x, rect.origin.y, flip_w, flip_h);

    out.primitives.push(primitive::outlined(
        card_r,
        4.0,
        if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        z,
    ));

    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            card_r.inflate(1.0),
            5.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    // 1. 翻页导航按钮（左 Prev，右 Next，28×28px 垂直居中）
    let btn_size = 28.0;
    let prev_x = card_r.origin.x + 8.0;
    let prev_y = card_r.origin.y + (flip_h - btn_size) * 0.5;
    let prev_btn_r = Rect::new(prev_x, prev_y, btn_size, btn_size);

    let next_x = card_r.max().x - 8.0 - btn_size;
    let next_y = prev_y;
    let next_btn_r = Rect::new(next_x, next_y, btn_size, btn_size);

    let btn_surface_token = if state.disabled {
        Token::NONE
    } else {
        Token::HOVER
    };
    out.primitives.push(primitive::surface(
        prev_btn_r,
        14.0,
        btn_surface_token,
        z + 1,
    ));
    out.primitives.push(primitive::surface(
        next_btn_r,
        14.0,
        btn_surface_token,
        z + 1,
    ));

    // 上一页箭头（因首页故为 TEXT_DISABLED）
    let prev_cx = prev_x + 14.0;
    let prev_cy = prev_y + 14.0;
    let arm = 4.0;
    primitive::curve_stroke(
        [prev_cx + 2.0, prev_cy - arm],
        [prev_cx - 2.0, prev_cy],
        1.5,
        Token::TEXT_DISABLED,
        z + 3,
        &mut out.primitives,
        &mut out.payloads,
    );
    primitive::curve_stroke(
        [prev_cx - 2.0, prev_cy],
        [prev_cx + 2.0, prev_cy + arm],
        1.5,
        Token::TEXT_DISABLED,
        z + 3,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 下一页箭头（活跃为 TEXT_PRIMARY，禁用为 TEXT_DISABLED）
    let next_cx = next_x + 14.0;
    let next_cy = next_y + 14.0;
    let next_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    primitive::curve_stroke(
        [next_cx - 2.0, next_cy - arm],
        [next_cx + 2.0, next_cy],
        1.5,
        next_ink,
        z + 3,
        &mut out.primitives,
        &mut out.payloads,
    );
    primitive::curve_stroke(
        [next_cx + 2.0, next_cy],
        [next_cx - 2.0, next_cy + arm],
        1.5,
        next_ink,
        z + 3,
        &mut out.primitives,
        &mut out.payloads,
    );

    // 2. 当前页内容排版（"Page 1" 居中）
    let current_page = CANONICAL_FLIP_PAGES[0];
    let page_ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let quads = text_ctx.shape_line_styled(
        current_page,
        card_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Center,
        AlignY::Middle,
    );
    primitive::glyphs(
        &quads,
        page_ink,
        z + 2,
        &mut out.primitives,
        &mut out.payloads,
    );

    FlipViewFixtureOutput {
        output: out,
        card_rect: card_r,
        prev_button_rect: prev_btn_r,
        next_button_rect: next_btn_r,
        total_size: Vec2::new(flip_w, flip_h),
    }
}

/// 发射标准化下拉刷新容器插桩（顶部环形指示器 + 下方内容卡片）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 260px、高 160px；
/// - 顶部包含 24px 直径的 ProgressRing 环形指示器（圆心位于水平居中、距顶 16px 处）：
///   - 未禁用时：绘制环形底轨、内部掏空与右上四分之一强调色圆弧（[`Token::PRIMARY`]，线宽 2.5px）；
///   - 禁用时：抑制强调色圆弧，底轨降级为变灰底色；
/// - 下方为内容卡片（宽 252px、高 120px，圆角 8px）：
///   - 位于 y = 36px 处，背景为 [`Token::CARD_BG`]、边框为 [`Token::CARD_BORDER`]；
///   - 内部左内边距 16px、垂直居中排版文本 `"Pull to refresh"`（字号 14px，字重 400）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，指示器与卡片内容全量半透明变灰衰减。
pub fn emit_canonical_refresh_fixture(
    rect: Rect,
    state: FixtureState,
    text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> RefreshFixtureOutput {
    let mut out = WidgetOutput::default();
    let refresh_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_REFRESH_WIDTH
    };
    let refresh_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_REFRESH_HEIGHT
    };
    let refresh_rect = Rect::new(rect.origin.x, rect.origin.y, refresh_w, refresh_h);

    // 1. 顶部 ProgressRing 环形指示器（直径 24px，水平居中）
    let ring_size = STANDARD_REFRESH_INDICATOR_SIZE;
    let ring_cx = refresh_rect.origin.x + refresh_w * 0.5;
    let ring_cy = refresh_rect.origin.y + 16.0;
    let ring_rect = Rect::new(
        ring_cx - ring_size * 0.5,
        ring_cy - ring_size * 0.5,
        ring_size,
        ring_size,
    );

    // 环形底轨
    out.primitives.push(primitive::surface(
        ring_rect,
        ring_size * 0.5,
        Token::SURFACE_SUNKEN,
        z + 1,
    ));
    // 内部掏空
    out.primitives.push(primitive::surface(
        ring_rect.inflate(-2.5),
        (ring_size - 5.0) * 0.5,
        if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        z + 2,
    ));

    // 弧形强调指示段（右上四分之一弧）
    if !state.disabled {
        let arc_r = 10.0;
        let p0 = [ring_cx, ring_cy - arc_r];
        let p1 = [ring_cx + arc_r * 0.707, ring_cy - arc_r * 0.707];
        let p2 = [ring_cx + arc_r, ring_cy];
        primitive::curve_stroke(
            p0,
            p1,
            2.5,
            Token::PRIMARY,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
        primitive::curve_stroke(
            p1,
            p2,
            2.5,
            Token::PRIMARY,
            z + 3,
            &mut out.primitives,
            &mut out.payloads,
        );
    }

    // 2. 下方内容卡片（圆角 8px）
    let card_y = refresh_rect.origin.y + 36.0;
    let card_h = (refresh_h - 40.0).max(1.0);
    let card_rect = Rect::new(refresh_rect.origin.x + 4.0, card_y, refresh_w - 8.0, card_h);

    out.primitives.push(primitive::outlined(
        card_rect,
        8.0,
        if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CARD_BG
        },
        if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::CARD_BORDER
        },
        1.0,
        0.0,
        z + 1,
    ));

    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            card_rect.inflate(1.0),
            9.0,
            Token::FOCUS_RING,
            2.0,
            z + 10,
            true,
        ));
    }

    // 3. 内容卡片文本
    let ink = if state.disabled {
        Token::TEXT_DISABLED
    } else {
        Token::TEXT_PRIMARY
    };
    let label_r = Rect::new(
        card_rect.origin.x + 16.0,
        card_rect.origin.y,
        (card_rect.width() - 32.0).max(1.0),
        card_rect.height(),
    );
    let quads = text_ctx.shape_line_styled(
        CANONICAL_REFRESH_TEXT,
        label_r,
        STANDARD_FONT_SIZE,
        400,
        None,
        AlignX::Left,
        AlignY::Middle,
    );
    primitive::glyphs(&quads, ink, z + 4, &mut out.primitives, &mut out.payloads);

    RefreshFixtureOutput {
        output: out,
        indicator_rect: ring_rect,
        card_rect,
        total_size: Vec2::new(refresh_w, refresh_h),
    }
}

/// 发射标准化标注滚动条插桩（8px 居中垂直轨道 + 32px 滑块 + 刻度线与标签）。
///
/// 几何与排版规则：
/// - 固有尺寸宽 16px、高 160px；
/// - 轨道宽度 8px，水平居中于 16px 控件包围盒内（左右各留 4px 边距），圆角 4px；
/// - 滑块位于进度 0.4 处，高度 32px，宽度 8px，圆角 3px：
///   - 未禁用时：使用滑块色令牌 [`Token::CONTROL_TRACK_STROKE`]；
///   - 禁用时：降级为变灰滑块令牌 [`Token::CONTROL_DISABLED_THUMB`]；
/// - 沿轨道排布 5 个等分刻度线（高度 1px，线宽 4px，水平居中于轨道）；
/// - 递归禁用态传播：当 `state.disabled` 为 true 时，轨道、滑块与全部刻度同步变灰。
pub fn emit_canonical_annotated_scroll_bar_fixture(
    rect: Rect,
    state: FixtureState,
    _text_ctx: &mut TextCtx<'_>,
    z: u16,
) -> AnnotatedScrollBarFixtureOutput {
    let mut out = WidgetOutput::default();
    let bar_w = if rect.width() > 0.0 {
        rect.width()
    } else {
        STANDARD_SCROLL_BAR_WIDTH
    };
    let bar_h = if rect.height() > 0.0 {
        rect.height()
    } else {
        STANDARD_SCROLL_BAR_HEIGHT
    };

    // 1. 垂直轨道（8px 居中）
    let track_w = STANDARD_SCROLL_BAR_TRACK_WIDTH;
    let track_x = rect.origin.x + (bar_w - track_w) * 0.5;
    let track_rect = Rect::new(track_x, rect.origin.y, track_w, bar_h);

    out.primitives.push(primitive::surface(
        track_rect,
        4.0,
        if state.disabled {
            Token::CONTROL_DISABLED_BG
        } else {
            Token::CONTROL_FILL
        },
        z + 1,
    ));

    // 2. 沿轨道的 5 个刻度线
    for i in 0..5 {
        let tick_y = rect.origin.y + i as f32 * ((bar_h - 2.0) / 4.0) + 1.0;
        let tick_r = Rect::new(track_x + 2.0, tick_y, 4.0, 1.0);
        let tick_token = if state.disabled {
            Token::CONTROL_DISABLED_BORDER
        } else {
            Token::DIVIDER
        };
        out.primitives
            .push(primitive::surface(tick_r, 0.0, tick_token, z + 2));
    }

    // 3. 滑块（0.4 位置，高度 32px）
    let thumb_h = 32.0;
    let thumb_y = rect.origin.y + (bar_h - thumb_h) * 0.4;
    let thumb_rect = Rect::new(track_x, thumb_y, track_w, thumb_h);

    let thumb_token = if state.disabled {
        Token::CONTROL_DISABLED_THUMB
    } else {
        Token::CONTROL_TRACK_STROKE
    };
    out.primitives
        .push(primitive::surface(thumb_rect, 3.0, thumb_token, z + 4));

    if state.focused && !state.disabled {
        out.primitives.push(primitive::focus_ring(
            thumb_rect.inflate(1.0),
            4.0,
            Token::FOCUS_RING,
            2.0,
            z + 6,
            true,
        ));
    }

    AnnotatedScrollBarFixtureOutput {
        output: out,
        track_rect,
        thumb_rect,
        total_size: Vec2::new(bar_w, bar_h),
    }
}

/// 通用递归禁用态变灰与衰减映射器。
///
/// 针对颜色令牌执行语义衰减：
/// - 文本前景色统一映射为 [`Token::TEXT_DISABLED`]；
/// - 交互高亮与强调色映射为抑制态或 [`Token::CONTROL_DISABLED_THUMB`]；
/// - 悬停/选中背景映射为 [`Token::NONE`]；
/// - 边框映射为 [`Token::CONTROL_DISABLED_BORDER`]。
#[must_use]
pub fn remap_token_to_disabled(token: Token) -> Token {
    match token {
        Token::TEXT_PRIMARY
        | Token::TEXT_SECONDARY
        | Token::ON_SURFACE
        | Token::ON_PRIMARY
        | Token::ON_ACCENT
        | Token::ON_SELECTION => Token::TEXT_DISABLED,

        Token::PRIMARY
        | Token::ACCENT
        | Token::PRIMARY_HOVER
        | Token::PRIMARY_PRESSED
        | Token::CONTROL_TRACK_STROKE => Token::CONTROL_DISABLED_THUMB,

        Token::HOVER
        | Token::PRESSED
        | Token::NAV_SELECTED
        | Token::NAV_HOVER
        | Token::FOCUS_RING
        | Token::PRIMARY_SOFT => Token::NONE,

        Token::CONTROL_STROKE | Token::CARD_BORDER | Token::DIVIDER => {
            Token::CONTROL_DISABLED_BORDER
        }

        other => other,
    }
}

/// 通用递归禁用态图元后处理函数。
///
/// 遍历并重写 `output` 中所有图元的颜色令牌与不透明度：
/// 1. 将所有文本、图标、曲线与高亮图元的颜色令牌递归映射为变灰/衰减令牌；
/// 2. 抑制悬停、选中背景与焦点环；
/// 3. 若 `attenuate_opacity` 为 true，将所有图元的不透明度乘以 [`WINUI_DISABLED_OPACITY`]。
pub fn apply_recursive_disabled_propagation(output: &mut WidgetOutput, attenuate_opacity: bool) {
    for prim in &mut output.primitives {
        let prim_type = prim.prim_type();

        // 1. 字形文本与彩色字形处理
        if matches!(prim_type, PrimType::Glyph | PrimType::GlyphColor) {
            let token = Token::new(prim.fill_token_or_uv as u16);
            let disabled_token = remap_token_to_disabled(token);
            prim.fill_token_or_uv = u32::from(disabled_token.raw());
        } else if prim_type == PrimType::Curve {
            // 2. 曲线描边（指示箭头、引导线等）
            let token = Token::new(prim.fill_token_or_uv as u16);
            let disabled_token = remap_token_to_disabled(token);
            prim.fill_token_or_uv = u32::from(disabled_token.raw());
        } else if matches!(prim_type, PrimType::RoundedRect | PrimType::Stroke) {
            // 3. 表面与边框处理
            let fill = Token::new(prim.fill_token_or_uv as u16);
            let border = Token::new(prim.border_token_or_tex as u16);

            let new_fill = remap_token_to_disabled(fill);
            let new_border = remap_token_to_disabled(border);

            prim.fill_token_or_uv = u32::from(new_fill.raw());
            prim.border_token_or_tex = u32::from(new_border.raw());
        }

        // 4. 不透明度衰减
        if attenuate_opacity {
            let cur_opacity = prim.opacity();
            let new_opacity = cur_opacity * WINUI_DISABLED_OPACITY;
            prim.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(prim.depth_u16(), new_opacity);
        }
    }
}
