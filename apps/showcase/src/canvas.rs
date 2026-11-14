//! 展厅外壳的绘制表面。
//!
//! [`Canvas`] 是唯一了解图元发射、裁剪、命中区域与语义的模块。
//! 页面调用其高层方法，从不直接接触 `GpuPrimitiveInstance`，从而使外壳代码
//! 保持清晰，并将事件/几何连线集中在一处。
//!

use accesskit::Role;
use rustc_hash::{FxHashMap, FxHashSet};
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{GpuClipNode, GpuPrimitiveInstance, CLIP_NONE};
use unbound_input::{ClipRect, CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphQuad, TextStyle};
use unbound_widgets::{ControlState, PointerPhase, WidgetCatalogId, WidgetOutput};

use crate::gallery::{Gallery, Labels, Palette};
use crate::target::{self, Target, ROOT};

/// 框内水平对齐方式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignX {
    Start,
    Center,
    End,
}

/// 框内垂直对齐方式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignY {
    Top,
    Center,
    Bottom,
}

/// 单个标识的悬停 / 按压状态。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Visual {
    Idle,
    Hover,
    Pressed,
    Disabled,
}

impl Visual {
    #[must_use]
    pub fn is_hovered(self) -> bool {
        matches!(self, Self::Hover)
    }
    #[must_use]
    pub fn is_pressed(self) -> bool {
        matches!(self, Self::Pressed)
    }
    #[must_use]
    pub fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// 单次页面构建的产出。
pub struct CanvasOutput {
    pub regions: Vec<HitRegion>,
    pub semantics: Semantics,
    pub focus_order: Vec<u64>,
    pub scroll_zones: Vec<(Rect, u8)>,
    pub ranges: Vec<RangeZone>,
    pub glyph_count: usize,
    /// 每个控件拥有的图元范围 `[start, end)`。
    ///
    /// 控件的图元是连续发射的，因此该跨度使上传路径能够识别脏切片属于*哪个控件*——
    /// 这也是悬停修补能够被证明为局部更新的原因。
    ///
    pub node_spans: FxHashMap<u64, (u32, u32)>,
    /// 本帧实际运行的真实控件发射器（覆盖率门禁）。
    pub emitted_widgets: FxHashSet<WidgetCatalogId>,
}

/// 类似滑块的区域，其值由指针的 x 坐标推导。
#[derive(Copy, Clone, Debug)]
pub struct RangeZone {
    pub node: u64,
    pub rect: Rect,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

/// 圆角裁剪形状：矩形及其圆角半径。
///
/// 这是 `GpuClipNode` 所携带的两个圆角槽位在 CPU 侧的孪生结构。
///
#[derive(Copy, Clone, Debug)]
struct RoundedSlot {
    rect: Rect,
    radii: [f32; 4],
}

/// 场景绘制器。
pub struct Canvas<'a> {
    pub scene: &'a mut Gallery,
    pub labels: &'a mut Labels,
    pub palette: &'a Palette,
    pub viewport: Rect,
    regions: Vec<HitRegion>,
    semantics: Semantics,
    focus_order: Vec<u64>,
    a11y_nodes: Vec<u64>,
    scroll_zones: Vec<(Rect, u8)>,
    ranges: Vec<RangeZone>,
    glyphs_emitted: usize,
    focused: Option<u64>,
    hover: Option<u64>,
    pressed: Option<u64>,
    z: u16,
    clips: Vec<u32>,
    clip_rects: Vec<ClipRect>,
    /// 当前层级继承的圆角裁剪槽位。
    ///
    /// 槽位 0 为最内层的圆角祖先，槽位 1 为其上一层——与着色器评估的双槽位模型一致。
    /// 将它们保存在栈中（而非塞进 CPU 镜像的单个 `ClipRect` 中），意味着中间插入的
    /// 普通轴对齐裁剪不再会抹除圆角祖先。
    ///
    ///
    rounded_stack: Vec<[Option<RoundedSlot>; 2]>,
    scale: f32,
    snap: bool,
    static_ids: u64,
    /// 每个控件拥有的图元范围。
    node_spans: FxHashMap<u64, (u32, u32)>,
    /// 本帧实际运行的真实 `unbound-widgets` 发射器。
    ///
    /// 编译期覆盖率门禁将每个 [`WidgetCatalogId`] 映射到演示；该集合则是运行时的半边证明——
    /// 仅手绘矩形的演示绝不会插入其 ID，因此覆盖率测试能够区分真实发射与伪造。
    ///
    ///
    emitted_widgets: FxHashSet<WidgetCatalogId>,
}

impl<'a> Canvas<'a> {
    /// 在空场景上创建绘制器。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scene: &'a mut Gallery,
        labels: &'a mut Labels,
        palette: &'a Palette,
        viewport: (f32, f32),
        focused: Option<u64>,
        hover: Option<u64>,
        pressed: Option<u64>,
        scale: f32,
        snap: bool,
    ) -> Self {
        let viewport = Rect::new(0.0, 0.0, viewport.0, viewport.1);
        let mut semantics = Semantics::new(ROOT);
        semantics.upsert(
            ROOT,
            A11yNode::new(Role::Window, "Unbound UI Gallery", viewport),
        );
        Self {
            scene,
            labels,
            palette,
            viewport,
            regions: Vec::new(),
            semantics,
            focus_order: Vec::new(),
            a11y_nodes: Vec::new(),
            scroll_zones: Vec::new(),
            ranges: Vec::new(),
            glyphs_emitted: 0,
            focused,
            hover,
            pressed,
            z: 1,
            clips: Vec::new(),
            clip_rects: Vec::new(),
            rounded_stack: Vec::new(),
            scale: scale.max(0.25),
            snap,
            static_ids: 0,
            node_spans: FxHashMap::default(),
            emitted_widgets: FxHashSet::default(),
        }
    }

    /// 下一个图元将占用的索引。
    ///
    /// 手绘控件在发射第一个图元前读取此索引，并在最后一个图元后将其传给
    /// [`record_span`](Self::record_span)，从而与发射器控件一样加入跨度映射表。
    ///
    #[must_use]
    pub fn span_start(&self) -> usize {
        self.scene.primitives.len()
    }

    /// 将自 `start` 起发射的图元记录为属于 `node`。
    pub fn record_span(&mut self, node: u64, start: usize) {
        let end = self.scene.primitives.len();
        if end > start {
            self.node_spans.insert(node, (start as u32, end as u32));
        }
    }

    /// `node` 当前是否拥有悬停高亮。
    #[must_use]
    pub fn hovered(&self, node: u64) -> bool {
        self.hover == Some(node)
    }

    /// `node` 当前是否拥有按压高亮。
    #[must_use]
    pub fn pressed(&self, node: u64) -> bool {
        self.pressed == Some(node)
    }

    /// 当前绘制深度。
    #[must_use]
    pub fn z(&self) -> u16 {
        self.z
    }

    /// 预留下一个绘制深度。
    pub fn next_z(&mut self) -> u16 {
        let z = self.z;
        self.z = self.z.saturating_add(1);
        z
    }

    /// 将逻辑坐标对齐到物理像素网格。
    #[must_use]
    pub fn snap_value(&self, value: f32) -> f32 {
        if self.snap {
            (value * self.scale).round() / self.scale
        } else {
            value
        }
    }

    /// 每逻辑单位对应的物理像素数。
    ///
    /// 必须保持精确 1 物理像素粗细的几何体（如窗口控件描边）需要此值来转换线宽。
    ///
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 当前裁剪矩形（如有）。
    #[must_use]
    pub fn clip_rect(&self) -> Option<Rect> {
        self.clip_rects.last().map(|clip| clip.rect)
    }

    /// 当前裁剪形状（含圆角）。
    #[must_use]
    pub fn clip_shape(&self) -> Option<ClipRect> {
        self.clip_rects.last().copied()
    }

    /// 开启一个裁剪区域。新裁剪会与当前裁剪求交。
    pub fn begin_clip(&mut self, rect: Rect) {
        self.push_clip(rect, None);
    }

    /// 开启一个由圆角矩形界定的裁剪区域。
    ///
    /// 超级着色器（uber shader）会评估两个圆角父级槽位，因此嵌套的 `begin_clip`
    /// （轴对齐卡片）与 `begin_clip_rounded`（其内部的首屏表面）可保留两种形状，
    /// 而不会退化为普通裁剪矩形。
    ///
    pub fn begin_clip_rounded(&mut self, rect: Rect, radius: f32) {
        self.push_clip(rect, Some(radius));
    }

    fn push_clip(&mut self, rect: Rect, radius: Option<f32>) {
        let inherited = self.clip_rects.last().copied();
        let combined = match inherited {
            Some(existing) => existing.rect.intersect(&rect),
            None => rect,
        };
        let mut node = GpuClipNode::from_aabb([
            combined.origin.x,
            combined.origin.y,
            combined.max().x,
            combined.max().y,
        ]);

        // 圆角祖先保存在各自独立的栈中，而非塞进单个 CPU 镜像矩形。
        //
        // 旧代码通过 `ClipRect` 继承父级圆角半径，中间插入普通轴对齐裁剪会将其重置为零——
        // 导致第三层完全丢失第一层的圆角形状。GPU 随后回退为裁剪矩形，圆角卡片便长出了直角。
        //
        //
        //
        //
        //
        // 槽位 0 为最内层圆角祖先，槽位 1 为其上一层：
        // 与着色器的双槽位遮罩评估完全一致，也与 `unbound-world` 使用的顺序相同。
        //
        let mut slots = self.rounded_stack.last().copied().unwrap_or([None, None]);
        if let Some(value) = radius {
            if value > 0.0 {
                slots = [
                    Some(RoundedSlot {
                        rect,
                        radii: [value.max(0.0); 4],
                    }),
                    slots[0],
                ];
            }
        }
        if let Some(slot) = slots[0] {
            Self::write_rounded_slot(
                &mut node.rounded_clip_0,
                &mut node.radii_0,
                slot.rect,
                slot.radii,
            );
        }
        if let Some(slot) = slots[1] {
            Self::write_rounded_slot(
                &mut node.rounded_clip_1,
                &mut node.radii_1,
                slot.rect,
                slot.radii,
            );
        }

        let radii = match radius {
            Some(value) => [value.max(0.0); 4],
            None => slots[0]
                .filter(|slot| slot.rect == combined)
                .map_or([0.0; 4], |slot| slot.radii),
        };
        let index = self.scene.clips.len() as u32;
        self.scene.clips.push(node);
        self.clips.push(index);
        self.clip_rects.push(ClipRect {
            rect: combined,
            radii,
        });
        self.rounded_stack.push(slots);
    }

    fn write_rounded_slot(
        center: &mut [f32; 4],
        packed: &mut [u16; 4],
        rect: Rect,
        radii: [f32; 4],
    ) {
        let middle = rect.center();
        *center = [middle.x, middle.y, rect.size.x * 0.5, rect.size.y * 0.5];
        *packed = unbound_math::f16::pack_radii(radii);
    }

    /// 结束最内层裁剪。
    pub fn end_clip(&mut self) {
        self.clips.pop();
        self.clip_rects.pop();
        self.rounded_stack.pop();
    }

    fn clip_index(&self) -> u32 {
        self.clips.last().copied().unwrap_or(CLIP_NONE)
    }

    fn apply_clip(&mut self, from: usize) {
        let index = self.clip_index();
        if index == CLIP_NONE {
            return;
        }
        for primitive in &mut self.scene.primitives[from..] {
            primitive.clip_idx = index;
        }
    }

    /// 解析某个标识的悬停 / 按压状态。
    #[must_use]
    pub fn visual(&self, node: u64, disabled: bool) -> Visual {
        if disabled {
            return Visual::Disabled;
        }
        if self.pressed == Some(node) {
            Visual::Pressed
        } else if self.hover == Some(node) {
            Visual::Hover
        } else {
            Visual::Idle
        }
    }

    /// 某个标识是否拥有键盘焦点。
    #[must_use]
    pub fn focused(&self, node: u64) -> bool {
        self.focused == Some(node)
    }

    /// 绘制圆角矩形并返回其深度。
    #[allow(clippy::too_many_arguments)]
    pub fn rect(
        &mut self,
        rect: Rect,
        radius: f32,
        fill: u16,
        border: u16,
        border_width: f32,
        shadow: f32,
    ) -> u16 {
        let z = self.next_z();
        // 对两条边同时对齐，绝不分别对原点和尺寸取整：后者可能把 1.0 宽的细线舍入为零。
        //
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        let start = self.scene.primitives.len();
        self.scene.rect(
            x0,
            y0,
            x1 - x0,
            y1 - y0,
            radius,
            fill,
            border,
            border_width,
            shadow,
            z,
        );
        self.apply_clip(start);
        z
    }

    /// 以指定不透明度绘制圆角矩形。
    ///
    /// 该图元在所有状态下都存在，变化的只是其不透明度。这使控件的图元数量保持恒定，
    /// 从而让状态切换可以通过修补缓冲池中的单个切片来完成，而无需全量重传场景。
    ///
    ///
    pub fn rect_faded(&mut self, rect: Rect, radius: f32, fill: u16, opacity: f32) -> u16 {
        let start = self.scene.primitives.len();
        let z = self.rect(rect, radius, fill, 0, 0.0, 0.0);
        self.fade(start, opacity);
        z
    }

    /// 以指定不透明度绘制描边。
    ///
    /// 键盘焦点环对应的 [`rect_faded`](Self::rect_faded) 伴生方法：
    /// 焦点环始终发射，失去焦点时仅将其淡出，而非移除图元。
    ///
    pub fn stroke_faded(
        &mut self,
        rect: Rect,
        radius: f32,
        color: u16,
        width: f32,
        opacity: f32,
    ) -> u16 {
        let start = self.scene.primitives.len();
        let z = self.stroke(rect, radius, color, width);
        self.fade(start, opacity);
        z
    }

    /// 设置 `start` 处图元的不透明度。
    fn fade(&mut self, start: usize, opacity: f32) {
        if let Some(primitive) = self.scene.primitives.get_mut(start) {
            primitive.z_and_opacity = unbound_gpu::data::GpuPrimitiveInstance::pack_z_opacity(
                primitive.depth_u16(),
                opacity,
            );
        }
    }

    /// 绘制仅含阴影的圆角矩形。
    pub fn shadow(&mut self, rect: Rect, radius: f32, blur: f32) -> u16 {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        self.scene
            .rect(x0, y0, x1 - x0, y1 - y0, radius, 0, 0, 0.0, blur, z);
        if let Some(prim) = self.scene.primitives.get_mut(start) {
            prim.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
                unbound_gpu::data::PrimType::Shadow,
                unbound_gpu::data::BlendMode::Normal,
                unbound_gpu::data::SubElement::Shadow as u16,
            );
        }
        self.apply_clip(start);
        z
    }

    /// 绘制空心圆角矩形。
    pub fn stroke(&mut self, rect: Rect, radius: f32, color: u16, width: f32) -> u16 {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        let x0 = self.snap_value(rect.origin.x);
        let y0 = self.snap_value(rect.origin.y);
        let x1 = self.snap_value(rect.max().x);
        let y1 = self.snap_value(rect.max().y);
        self.scene
            .stroke(x0, y0, x1 - x0, y1 - y0, radius, color, z);
        if let Some(prim) = self.scene.primitives.get_mut(start) {
            prim.border_width = unbound_math::f16::f32_to_f16(width);
        }
        self.apply_clip(start);
        z
    }

    /// 在 `rect` 内绘制单行标签。
    ///
    /// 垂直放置遵循**基线（baseline）**而非文本段的墨水框（ink box）：
    /// 两个同字号标签必须共享同一基线，否则一排按钮中的 `"Light"` 会比 `"Dark"`
    /// 高出几个像素——因为墨水框取决于字符串自身最高的字形。
    ///
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        color: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Rect {
        let (quads, ink) = self.layout_text(text, rect, size, align_x, align_y);
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in &quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
        ink
    }

    /// 在对齐位置塑形一段文本，**不**进行绘制。
    ///
    /// 标准控件自行绘制标签，因此外壳需要精确位于绘制器所用位置上的已塑形字形——
    /// 由基线而非墨水框决定行高位置。
    ///
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text(
        &mut self,
        text: &str,
        rect: Rect,
        size: f32,
        align_x: AlignX,
        align_y: AlignY,
    ) -> (Vec<GlyphQuad>, Rect) {
        let metrics = self.labels.metrics(text, size);
        let x = match align_x {
            AlignX::Start => rect.origin.x,
            AlignX::Center => rect.origin.x + (rect.size.x - metrics.width) * 0.5,
            AlignX::End => rect.max().x - metrics.width,
        };
        // 参与对齐的是行框：其顶部对应框顶，底部对应框底，中心对应框中。
        //
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            AlignY::Center => rect.origin.y + (rect.size.y - metrics.line_height()) * 0.5,
            AlignY::Bottom => rect.max().y - metrics.line_height(),
        };
        // 对齐运行段原点而非每个字形：在相位 0 下笔触步进为整数，
        // 因此对齐一个原点即可在不改变字距的前提下让整行落在像素网格上。
        //
        let x = self.snap_value(x);
        let y = self.snap_value(y);
        let quads = self.labels.layout(text, Vec2::new(x, y), size);
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, metrics.width, size));
        (quads, ink)
    }

    /// 在对齐位置塑形带样式的文本段，不进行绘制。
    ///
    /// 与 [`Self::layout_text`] 采用相同的对齐规则，但显式指定字重、倾斜与字体族：
    /// 字体检查器的核心目的正是展示它们之间的差异。
    ///
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled(
        &mut self,
        text: &str,
        rect: Rect,
        style: &TextStyle,
        align_x: AlignX,
        align_y: AlignY,
    ) -> (Vec<GlyphQuad>, Rect) {
        let metrics = self.labels.metrics_styled(text, style);
        let x = match align_x {
            AlignX::Start => rect.origin.x,
            AlignX::Center => rect.origin.x + (rect.size.x - metrics.width) * 0.5,
            AlignX::End => rect.max().x - metrics.width,
        };
        let y = match align_y {
            AlignY::Top => rect.origin.y,
            AlignY::Center => rect.origin.y + (rect.size.y - metrics.line_height()) * 0.5,
            AlignY::Bottom => rect.max().y - metrics.line_height(),
        };
        let x = self.snap_value(x);
        let y = self.snap_value(y);
        let quads = self.labels.layout_styled(text, Vec2::new(x, y), style);
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, metrics.width, style.font_size));
        (quads, ink)
    }

    /// 在 `rect` 内绘制带样式的单行标签。
    #[allow(clippy::too_many_arguments)]
    pub fn text_styled(
        &mut self,
        text: &str,
        rect: Rect,
        style: &TextStyle,
        color: u16,
        align_x: AlignX,
        align_y: AlignY,
    ) -> Rect {
        let (quads, ink) = self.layout_text_styled(text, rect, style, align_x, align_y);
        self.glyphs(&quads, color);
        ink
    }

    /// 标准控件的视觉状态。
    ///
    /// 外壳已从路由器解析出悬停、按压与焦点；控件层只需按其理解的形式接收即可。
    ///
    #[must_use]
    pub fn control_state(&self, node: u64, disabled: bool, selected: bool) -> ControlState {
        let phase = if disabled {
            PointerPhase::Idle
        } else if self.pressed == Some(node) {
            PointerPhase::Pressed
        } else if self.hover == Some(node) {
            PointerPhase::Hovered
        } else {
            PointerPhase::Idle
        };
        ControlState {
            phase,
            focused: self.focused == Some(node),
            disabled,
            selected,
        }
    }

    /// 绘制标准控件的输出。
    ///
    /// 控件的图元会重定基准到画布绘制深度——保持其相对顺序——并像其他图元一样接受裁剪。
    /// 其命中区域与语义节点会注册到画布账本中，使外壳在焦点顺序上保持单一事实来源。
    ///
    ///
    ///
    pub fn control_output(&mut self, node: u64, output: &WidgetOutput) {
        let start = self.scene.primitives.len();
        if node != 0 {
            self.record_span(node, start);
        }
        let payload_base = self.scene.payloads.len() as u32;
        for mut primitive in output.primitives.iter().copied() {
            primitive.z_and_opacity =
                GpuPrimitiveInstance::pack_z_opacity(self.next_z(), primitive.opacity());
            if payload_base != 0 {
                primitive.extra_payload_idx =
                    primitive.extra_payload_idx.saturating_add(payload_base);
            }
            self.scene.primitives.push(primitive);
        }
        self.scene.payloads.extend(output.payloads.iter().copied());
        self.glyphs_emitted += output.payloads.len();
        self.apply_clip(start);

        let clip_shape = self.clip_shape();
        if let Some(mut region) = output.hit.clone() {
            region.z = u32::from(self.z);
            if let Some(clip) = clip_shape {
                region = region.with_clip(clip);
            }
            self.regions.push(region);
        }
        let demo_parent = match crate::target::decode(node) {
            Some(crate::target::Target::Demo(page, card, _)) => Some((page, card)),
            _ => None,
        };
        for (idx, mut sub_region) in output.sub_hits.iter().cloned().enumerate() {
            if let Some((page, card)) = demo_parent {
                let same_demo = matches!(
                    crate::target::decode(sub_region.node),
                    Some(crate::target::Target::Demo(p, c, _)) if p == page && c == card
                );
                if !same_demo {
                    sub_region.node = crate::target::id(crate::target::Target::Demo(
                        page,
                        card,
                        1000_u16.saturating_add(idx as u16),
                    ));
                }
            }
            sub_region.z = u32::from(self.z);
            if let Some(clip) = clip_shape {
                sub_region = sub_region.with_clip(clip);
            }
            self.regions.push(sub_region);
        }

        if let Some(a11y) = output.a11y.clone() {
            if output.focusable {
                self.focus_order.push(node);
            }
            self.semantics.upsert(node, a11y);
            self.a11y_nodes.push(node);
        }
        for (sub_key, sub_a11y) in output.sub_a11y.iter().cloned() {
            self.semantics.upsert(sub_key, sub_a11y);
            if node != 0 {
                self.semantics.reparent(sub_key, node);
            }
            self.a11y_nodes.push(sub_key);
        }
        self.focus_order.extend_from_slice(&output.sub_focus_order);
    }

    /// 绘制真实组件的输出并记录其发射器已运行。
    ///
    /// 这是 [`control_output`](Self::control_output) 的已验证版本：
    /// 演示需指明其发射的 [`WidgetCatalogId`]，因此帧中的 `emitted_widgets` 集合能证明
    /// 确实运行了真实的 `unbound-widgets` 发射器而非手绘替身。
    ///
    pub fn widget_output(&mut self, id: WidgetCatalogId, node: u64, output: &WidgetOutput) {
        self.emitted_widgets.insert(id);
        self.control_output(node, output);
    }

    /// 本帧实际运行的真实组件发射器。
    #[must_use]
    pub fn emitted_widgets(&self) -> &FxHashSet<WidgetCatalogId> {
        &self.emitted_widgets
    }

    /// 对本帧截至目前已构建的实时语义树的只读访问。
    #[must_use]
    pub fn semantics(&self) -> &Semantics {
        &self.semantics
    }

    /// 为组件发射器将单行标签塑形为字形四边形。
    pub fn shape(&mut self, text: &str, origin: Vec2, size: f32) -> Vec<GlyphQuad> {
        self.labels.layout(text, origin, size)
    }

    /// 在 `rect` 内绘制自动换行段落。
    pub fn text_wrapped(&mut self, text: &str, rect: Rect, size: f32, color: u16) -> Rect {
        let x = self.snap_value(rect.origin.x);
        let y = self.snap_value(rect.origin.y);
        let quads = self
            .labels
            .layout_wrapped(text, Vec2::new(x, y), size, rect.size.x.max(1.0));
        let ink = quads_bounds(&quads).unwrap_or(Rect::new(x, y, 0.0, 0.0));
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in &quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
        ink
    }

    /// 在场景空间中绘制已在别处完成布局的字形。
    pub fn glyphs(&mut self, quads: &[GlyphQuad], color: u16) {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        for quad in quads {
            self.scene.glyph(quad, color, z);
        }
        self.glyphs_emitted += quads.len();
        self.apply_clip(start);
    }

    /// 绘制二次贝塞尔曲线段。
    pub fn curve(&mut self, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], width: f32, color: u16) {
        let z = self.next_z();
        let start = self.scene.primitives.len();
        self.scene.curve(p0, p1, p2, width, color, z);
        self.apply_clip(start);
    }

    /// 细线分隔符。
    pub fn divider(&mut self, x: f32, y: f32, width: f32) {
        self.rect(
            Rect::new(x, y, width, 1.0),
            0.0,
            self.palette.divider,
            0,
            0.0,
            0.0,
        );
    }

    /// 注册可点击区域及其语义节点。
    #[allow(clippy::too_many_arguments)]
    pub fn control(
        &mut self,
        node: u64,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        cursor: CursorKind,
        disabled: bool,
        selected: bool,
    ) {
        self.control_on_layer(node, rect, role, label, cursor, disabled, selected, 1);
    }

    /// 在显式传送门（portal）层上注册可点击区域。
    #[allow(clippy::too_many_arguments)]
    pub fn control_on_layer(
        &mut self,
        node: u64,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        cursor: CursorKind,
        disabled: bool,
        selected: bool,
        layer: u8,
    ) {
        let mut region = HitRegion::new(node, layer, u32::from(self.z), rect).with_cursor(cursor);
        if let Some(clip) = self.clip_shape() {
            region = region.with_clip(clip);
        }
        self.regions.push(region);

        let mut a11y = A11yNode::new(role, label, rect);
        if !disabled {
            a11y = a11y.clickable();
            self.focus_order.push(node);
        }
        a11y.disabled = disabled;
        a11y.selected = selected;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
    }

    /// 注册非交互式语义节点。
    pub fn semantic(&mut self, role: Role, label: impl Into<String>, rect: Rect) -> u64 {
        self.static_ids += 1;
        let node = 0x00f0_0000_0000_0000 | self.static_ids;
        let mut a11y = A11yNode::new(role, label, rect);
        a11y.focusable = false;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
        node
    }

    /// 注册标题语义节点。
    pub fn heading(&mut self, label: impl Into<String>, rect: Rect, level: u32) -> u64 {
        self.static_ids += 1;
        let node = 0x00f0_0000_0000_0000 | self.static_ids;
        let mut a11y = A11yNode::new(Role::Heading, label, rect);
        a11y.level = level;
        self.semantics.upsert(node, a11y);
        self.a11y_nodes.push(node);
        node
    }

    /// 注册非标准控件的演示部件。
    #[allow(clippy::too_many_arguments)]
    pub fn demo_control(
        &mut self,
        page: u8,
        card: u8,
        part: u16,
        rect: Rect,
        role: Role,
        label: impl Into<String>,
        selected: bool,
    ) -> u64 {
        let node = target::id(Target::Demo(page, card, part));
        self.control(
            node,
            rect,
            role,
            label,
            CursorKind::Pointer,
            false,
            selected,
        );
        node
    }

    /// 截至目前发射的字形。
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.glyphs_emitted
    }

    /// 声明一个由演示处理滚轮手势的区域。
    pub fn scroll_zone(&mut self, rect: Rect, tag: u8) {
        self.scroll_zones.push((rect, tag));
    }

    /// 声明一个值随指针水平位置变化的水平范围。
    pub fn range(&mut self, node: u64, rect: Rect, min: f32, max: f32, step: f32) {
        self.ranges.push(RangeZone {
            node,
            rect,
            min,
            max,
            step,
        });
    }

    /// 完成绘制并交回交互数据。
    #[must_use]
    pub fn finish(mut self) -> CanvasOutput {
        self.semantics.set_children(ROOT, &self.a11y_nodes);
        self.semantics.set_focus(self.focused.unwrap_or(ROOT));
        CanvasOutput {
            regions: self.regions,
            semantics: self.semantics,
            focus_order: self.focus_order,
            scroll_zones: self.scroll_zones,
            ranges: self.ranges,
            glyph_count: self.glyphs_emitted,
            node_spans: self.node_spans,
            emitted_widgets: self.emitted_widgets,
        }
    }
}

/// 字形运行段的墨水边界。
#[must_use]
pub fn quads_bounds(quads: &[GlyphQuad]) -> Option<Rect> {
    let first = quads.first()?;
    let mut bounds = first.rect;
    for quad in &quads[1..] {
        bounds = bounds.union(&quad.rect);
    }
    Some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gallery::{palette, Gallery, Labels};

    /// 基于空场景的画布。
    fn canvas<'a>(
        scene: &'a mut Gallery,
        labels: &'a mut Labels,
        palette: &'a Palette,
    ) -> Canvas<'a> {
        Canvas::new(
            scene,
            labels,
            palette,
            (400.0, 400.0),
            None,
            None,
            None,
            1.0,
            false,
        )
    }

    /// `index` 处裁剪节点的圆角半径。
    fn radii_of(node: &GpuClipNode, slot: usize) -> [f32; 4] {
        let packed = if slot == 0 {
            node.radii_0
        } else {
            node.radii_1
        };
        unbound_math::f16::unpack_radii(packed)
    }

    /// `index` 处裁剪节点的圆角矩形。
    fn rounded_of(node: &GpuClipNode, slot: usize) -> [f32; 4] {
        if slot == 0 {
            node.rounded_clip_0
        } else {
            node.rounded_clip_1
        }
    }

    #[test]
    fn a_third_level_plain_clip_keeps_the_rounded_ancestor() {
        let mut scene = Gallery::new();
        let mut labels = Labels::new();
        let palette = palette();
        let mut canvas = canvas(&mut scene, &mut labels, &palette);

        let outer = Rect::new(10.0, 10.0, 120.0, 120.0);
        canvas.begin_clip_rounded(outer, 12.0);
        canvas.begin_clip(Rect::new(20.0, 20.0, 80.0, 80.0));
        canvas.begin_clip(Rect::new(30.0, 30.0, 40.0, 40.0));

        let node = canvas.scene.clips.last().expect("three clip nodes");
        assert_eq!(
            radii_of(node, 0),
            [12.0; 4],
            "the innermost clip lost the rounded ancestor"
        );
        assert_eq!(
            rounded_of(node, 0),
            [70.0, 70.0, 60.0, 60.0],
            "the rounded slot must describe the outer rounded rectangle"
        );
    }

    #[test]
    fn a_nested_rounded_clip_keeps_both_shapes() {
        let mut scene = Gallery::new();
        let mut labels = Labels::new();
        let palette = palette();
        let mut canvas = canvas(&mut scene, &mut labels, &palette);

        canvas.begin_clip_rounded(Rect::new(0.0, 0.0, 100.0, 100.0), 12.0);
        canvas.begin_clip_rounded(Rect::new(10.0, 10.0, 40.0, 40.0), 4.0);

        let node = canvas.scene.clips.last().expect("two clip nodes");
        assert_eq!(
            radii_of(node, 0),
            [4.0; 4],
            "the inner shape is the first slot"
        );
        assert_eq!(
            radii_of(node, 1),
            [12.0; 4],
            "the outer shape is the second slot"
        );
    }
}
