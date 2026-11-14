//! 构建组件展厅场景。
//!
//! 此处的每个条目都对应超级着色器（uber shader）所支持的一种图元类型，
//! 因此窗口会一次性展示完整的渲染词汇表：圆角矩形、边框、解析阴影、曲线、
//! 图像与字形。
//!
//! 场景仅在离散交互状态变化（悬停 / 按压 / 焦点 / 点击计数）时重建，
//! 而非在画布相机移动时重建。平移、缩放与切换主题仅触及相机 uniform 或 4 KiB
//! 颜色表，因此图元缓冲区保持不变——这正是引擎对这些操作宣称“0 字节上传”的原因。
//!
//!

use accesskit::Role;
use unbound_a11y::{A11yNode, Semantics};
use unbound_gpu::data::{
    BlendMode, GpuClipNode, GpuPayload, GpuPrimitiveInstance, GpuTransform, PrimFlags, PrimType,
    SubElement,
};
use unbound_gpu::{Camera, SceneUpload};
use unbound_headless::{Frame, FrameBuilder, NodeKind};
use unbound_input::HitRegion;
use unbound_math::{Rect, Vec2};
use unbound_text::{GlyphCache, GlyphQuad, ShapedText, TextEngine, TextStyle};
use unbound_theme::Token;

use crate::button::{self, ButtonDesc, ButtonVisualState};

/// 展厅使用的色彩令牌。
///
/// 主题查找表（LUT）按主题一次性写入；图元仅存储这些索引。
///
///
/// 两个主题使用**完全相同**的槽位布局——仅颜色不同——这使得切换主题只需对表
/// 进行一次 4 KiB 覆写。槽位 0 保留给“无颜色”，槽位 1 为引擎的阴影槽位
/// （`common.glsl` 中的 `SHADOW_TOKEN`）。
///
#[derive(Copy, Clone, Debug)]
pub struct Palette {
    /// 页面背景。
    pub background: u16,
    /// 面板表面。
    pub surface: u16,
    /// 面板边框。
    pub border: u16,
    /// 主强调色。
    pub primary: u16,
    /// 次强调色。
    pub secondary: u16,
    /// 成功色。
    pub success: u16,
    /// 警告色。
    pub warning: u16,
    /// 危险色。
    pub danger: u16,
    /// 正文文本色。
    pub text: u16,
    /// 弱化文本色。
    pub text_muted: u16,
    /// 阴影色。
    pub shadow: u16,
    /// 悬停时的按钮填充。
    pub primary_hover: u16,
    /// 按压时的按钮填充。
    pub primary_pressed: u16,
    /// 按钮标签颜色。
    pub on_primary: u16,
    /// 焦点环颜色。
    pub focus_ring: u16,
    /// 禁用状态填充。
    pub disabled_fill: u16,
    /// 标题栏 / 全局页头表面。
    pub header_bg: u16,
    /// 导航窗格表面。
    pub nav_bg: u16,
    /// 悬停中的导航行。
    pub nav_hover: u16,
    /// 选中的导航行。
    pub nav_selected: u16,
    /// 示例卡片表面。
    pub card_bg: u16,
    /// 示例卡片描边。
    pub card_border: u16,
    /// 实时舞台背景。
    pub stage_bg: u16,
    /// 舞台的网格点。
    pub stage_grid: u16,
    /// 滑块 / 开关轨道。
    pub control_track: u16,
    /// 滑块拇指与开关旋钮。
    pub control_thumb: u16,
    /// 悬停中的控件表面。
    pub control_hover: u16,
    /// 代码面板背景。
    pub code_bg: u16,
    /// 代码面板前景。
    pub code_text: u16,
    /// 模态遮罩层（scrim）。
    pub overlay_scrim: u16,
    /// 浮层表面。
    pub overlay_bg: u16,
    /// 12% 透明度的主强调色。
    pub primary_soft: u16,
    /// 12% 透明度的成功色。
    pub success_soft: u16,
    /// 12% 透明度的警告色。
    pub warning_soft: u16,
    /// 12% 透明度的危险色。
    pub danger_soft: u16,
    /// 首屏渐变起点色。
    pub hero_start: u16,
    /// 首屏渐变终点色。
    pub hero_end: u16,
    /// 绘制在首屏渐变上的文本与图标色。
    pub on_hero: u16,
    /// 文本选区填充色。
    pub selection_fill: u16,
    /// 细线分隔符。
    pub divider: u16,
    /// 性能 HUD 背景。
    pub hud_bg: u16,
    /// 性能 HUD 前景。
    pub hud_text: u16,
    /// 12% 透明度的强调色。
    pub accent_soft: u16,
    /// 画布 / 舞台网格点。
    pub grid_dot: u16,
    /// 12% 透明度的次强调色。
    pub secondary_soft: u16,
    /// 导航主文本色。
    pub nav_text: u16,
    /// 导航次文本色。
    pub nav_text_muted: u16,
    /// 首屏表面上行动号召按钮背后的遮罩。
    pub hero_scrim: u16,
}

impl Palette {
    /// 设计指南页面展示的色板。
    #[must_use]
    pub fn swatches(&self) -> [(&'static str, u16); 12] {
        [
            ("Surface", self.surface),
            ("Raised", self.card_bg),
            ("Primary", self.primary),
            ("Primary soft", self.primary_soft),
            ("Accent", self.secondary),
            ("Success", self.success),
            ("Warning", self.warning),
            ("Danger", self.danger),
            ("Text", self.text),
            ("Muted", self.text_muted),
            ("Outline", self.border),
            ("Shadow", self.shadow),
        ]
    }
}

/// 两个主题共用的令牌布局。
///
/// | 槽位 | 含义 |
/// | ---: | :--- |
/// | 0 | 无（完全透明） |
/// | 1 | 阴影——由引擎的 `SHADOW_TOKEN` 固定 |
/// | 2 | 页面背景 |
/// | 3 | 面板表面 |
/// | 4 | 面板边框 |
/// | 5..9 | 主色、次色、成功、警告、危险 |
/// | 10 | 正文文本 |
/// | 11 | 弱化文本 |
/// | 12 | 主按钮——悬停 |
/// | 13 | 主按钮——按压 |
/// | 14 | 按钮标签（位于主按钮上） |
/// | 15 | 焦点环 |
/// | 16 | 禁用填充 |
/// 展厅使用的令牌。
///
/// 两个主题共享相同的槽位——仅表中的颜色不同——因此切换主题绝不会触及场景数据。
/// 槽位编号来自引擎的语义词汇表（[`unbound_theme::Token`]）；
/// 具体颜色则定义在 [`crate::theme`] 中。
///
#[must_use]
pub const fn palette() -> Palette {
    Palette {
        background: Token::SURFACE.raw(),
        surface: Token::SURFACE_RAISED.raw(),
        border: Token::OUTLINE.raw(),
        primary: Token::PRIMARY.raw(),
        secondary: Token::SECONDARY.raw(),
        success: Token::SUCCESS.raw(),
        warning: Token::WARNING.raw(),
        danger: Token::ERROR.raw(),
        text: Token::TEXT_PRIMARY.raw(),
        text_muted: Token::TEXT_SECONDARY.raw(),
        shadow: Token::SHADOW.raw(),
        primary_hover: Token::PRIMARY_HOVER.raw(),
        primary_pressed: Token::PRIMARY_PRESSED.raw(),
        on_primary: Token::ON_PRIMARY.raw(),
        focus_ring: Token::FOCUS_RING.raw(),
        disabled_fill: Token::DISABLED.raw(),
        header_bg: Token::HEADER_BG.raw(),
        nav_bg: Token::NAV_BG.raw(),
        nav_hover: Token::NAV_HOVER.raw(),
        nav_selected: Token::NAV_SELECTED.raw(),
        card_bg: Token::CARD_BG.raw(),
        card_border: Token::CARD_BORDER.raw(),
        stage_bg: Token::CANVAS_BACKDROP.raw(),
        stage_grid: Token::GRID.raw(),
        control_track: Token::CONTROL_TRACK.raw(),
        control_thumb: Token::CONTROL_THUMB.raw(),
        control_hover: Token::CONTROL_HOVER.raw(),
        code_bg: Token::CODE_BG.raw(),
        code_text: Token::CODE_TEXT.raw(),
        overlay_scrim: Token::SCRIM.raw(),
        overlay_bg: Token::OVERLAY_BG.raw(),
        primary_soft: Token::PRIMARY_SOFT.raw(),
        success_soft: Token::SUCCESS_SOFT.raw(),
        warning_soft: Token::WARNING_SOFT.raw(),
        danger_soft: Token::ERROR_SOFT.raw(),
        hero_start: Token::HERO_START.raw(),
        hero_end: Token::HERO_END.raw(),
        on_hero: Token::ON_HERO.raw(),
        selection_fill: Token::SELECTION.raw(),
        divider: Token::DIVIDER.raw(),
        hud_bg: Token::HUD_BG.raw(),
        hud_text: Token::HUD_TEXT.raw(),
        accent_soft: Token::ACCENT_SOFT.raw(),
        grid_dot: Token::GRID_DOT.raw(),
        secondary_soft: Token::SECONDARY_SOFT.raw(),
        nav_text: Token::NAV_TEXT.raw(),
        nav_text_muted: Token::NAV_TEXT_MUTED.raw(),
        hero_scrim: Token::HERO_SCRIM.raw(),
    }
}

/// 准备就绪待上传的展厅场景。
#[derive(Default)]
pub struct Gallery {
    /// 按绘制顺序排列的图元。
    pub primitives: Vec<GpuPrimitiveInstance>,
    /// 变换表；本场景仅使用单位变换。
    pub transforms: Vec<GpuTransform>,
    /// 裁剪节点。
    pub clips: Vec<GpuClipNode>,
    /// 曲线与字形的载荷数据。
    pub payloads: Vec<GpuPayload>,
    /// 相机。
    pub camera: Camera,
}

impl Gallery {
    /// 初始化空场景。
    pub(crate) fn new() -> Self {
        Self {
            primitives: Vec::new(),
            transforms: vec![GpuTransform::IDENTITY],
            clips: Vec::new(),
            payloads: Vec::new(),
            camera: Camera::ui((1280.0, 720.0), 1.0),
        }
    }

    /// 添加圆角矩形。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        fill: u16,
        border: u16,
        border_width: f32,
        shadow: f32,
        z: u16,
    ) {
        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: unbound_math::f16::pack_radii([radius; 4]),
            border_width: unbound_math::f16::f32_to_f16(border_width),
            shadow_blur: unbound_math::f16::f32_to_f16(shadow),
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: u32::from(fill),
            border_token_or_tex: u32::from(border),
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::RoundedRect,
                BlendMode::Normal,
                SubElement::Background as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: 0,
        };
        self.primitives.push(prim);
    }

    /// 添加描边（空心圆角矩形）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stroke(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: u16,
        z: u16,
    ) {
        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [x, y],
            size: [w, h],
            corner_radii: unbound_math::f16::pack_radii([radius; 4]),
            border_width: unbound_math::f16::f32_to_f16(2.0),
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: 0,
            border_token_or_tex: u32::from(color),
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                PrimType::Stroke,
                BlendMode::Normal,
                SubElement::Border as u16,
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: 0,
        };
        self.primitives.push(prim);
    }

    /// 添加字形四边形。
    pub(crate) fn glyph(&mut self, quad: &GlyphQuad, color: u16, z: u16) {
        let payload = GpuPayload {
            p0: [quad.uv[0], quad.uv[1]],
            p1: [quad.uv[2], quad.uv[3]],
            p2: [0.0, 0.0],
            line_width: 0.0,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        };
        let payload_idx = self.payloads.len() as u32;
        self.payloads.push(payload);

        let prim = GpuPrimitiveInstance {
            chunk_id: [0, 0],
            local_pos: [quad.rect.origin.x, quad.rect.origin.y],
            size: [quad.rect.width(), quad.rect.height()],
            corner_radii: [0; 4],
            border_width: 0,
            shadow_blur: 0,
            transform_idx: 0,
            clip_idx: unbound_gpu::data::CLIP_NONE,
            fill_token_or_uv: u32::from(color),
            // 图集图层复用边框槽位。
            border_token_or_tex: quad.layer,
            prim_type_and_flags: GpuPrimitiveInstance::pack_type(
                if quad.color {
                    PrimType::GlyphColor
                } else {
                    PrimType::Glyph
                },
                BlendMode::Normal,
                // 着色器必须知道 RGBA 纹素是距离场；
                // 若缺少该标志，它会将其当作普通彩色位图并绘制成不透明色块。
                //
                SubElement::Content as u16 | if quad.msdf { PrimFlags::MSDF.bits() } else { 0 },
            ),
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(z, 1.0),
            extra_payload_idx: payload_idx,
        };
        self.primitives.push(prim);
    }

    /// 添加二次曲线段。
    pub(crate) fn curve(
        &mut self,
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        line_width: f32,
        color: u16,
        z: u16,
    ) {
        let segment = unbound_vector::QuadSegment::new(
            Vec2::new(p0[0], p0[1]),
            Vec2::new(p1[0], p1[1]),
            Vec2::new(p2[0], p2[1]),
        );
        let instance = unbound_vector::CurveInstance::from_quad(&segment, line_width);
        let draw = unbound_gpu::CurveDraw::new([0, 0], &instance, u32::from(color), line_width, z);
        let mut prim = draw.primitive;
        prim.extra_payload_idx = self.payloads.len() as u32;
        prim.prim_type_and_flags = GpuPrimitiveInstance::pack_type(
            PrimType::Curve,
            BlendMode::Normal,
            SubElement::Content as u16,
        );
        // 控制点位于有向包围盒*自身*的坐标系中，因此包围盒的旋转必须传递给顶点着色器。
        // 若缺少旋转信息，曲线会按未旋转状态画在错误位置，
        // 并在从未旋转过的轴对齐包围盒边缘被裁剪。
        //
        let transform_idx = if draw.transform == GpuTransform::IDENTITY {
            0
        } else {
            let idx = self.transforms.len() as u32;
            self.transforms.push(draw.transform);
            idx
        };
        prim.transform_idx = transform_idx;
        self.payloads.push(draw.payload);
        self.primitives.push(prim);
    }

    /// 作为上传计划的场景。
    ///
    /// `dirty_ranges` 是自上次上传以来发生变化的图元范围。
    /// 空切片表示“全量上传”；非空切片则让渲染器仅修补这些切片，
    /// 从而将悬停交互转化为寥寥几个字节的上传。
    ///
    #[must_use]
    pub fn upload<'a>(
        &'a self,
        generation: u64,
        dirty_ranges: &'a [(u32, u32)],
    ) -> SceneUpload<'a> {
        SceneUpload {
            primitives: &self.primitives,
            transforms: &self.transforms,
            clips: &self.clips,
            payloads: &self.payloads,
            generation,
            camera: self.camera,
            key_prefix: (unbound_gpu::data::PassLayer::Ui as u64) << 56,
            dirty_ranges,
            aux_changed: false,
            backdrops: &[],
        }
    }
}

/// 已布局文本运行段的度量信息（逻辑单位）。
///
/// 垂直度量来自**字体**而非文本段的墨水边界。同字号的两个运行段必须共享基线——
/// 否则一排按钮中的 `"Comfortable"` 会比 `"Compact"` 低几个像素，
/// 因为后者没有降部（descender）而前者有升部（ascender）。
///
///
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TextMetrics {
    /// 最宽行的步进宽度。
    pub width: f32,
    /// 整个文本块的高度。
    pub height: f32,
    /// 从行框顶部到基线的距离。
    pub ascent: f32,
    /// 从基线到行框底部的距离。
    pub descent: f32,
    /// 首行基线相对于行框顶部的位置。
    pub first_baseline: f32,
    /// 运行段占用的行数。
    pub line_count: u32,
}

impl TextMetrics {
    /// 单个行框的高度。
    #[must_use]
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// 驱动展厅标签的文本引擎。
///
/// `TextEngine` 同时负责塑形并持有两级缓存；其上的 `GlyphCache` 负责光栅化字形
/// 并将其保存在图集中。
pub struct Labels {
    /// 字形缓存。
    pub cache: GlyphCache,
    /// 塑形引擎。
    pub engine: TextEngine,
    /// 每个逻辑单位覆盖的图集纹素数。
    ///
    /// 布局保持在逻辑单位；按 `font_size × raster_scale` 光栅化并将四边形缩回，
    /// 可在 OS 缩放或应用缩放放大整个 UI 时保持每个字形锐利清晰。
    ///
    raster_scale: f32,
    /// 动态按钮标签预热时所使用的字号与光栅化缩放。
    prewarmed: Option<(f32, f32)>,
}

impl Labels {
    /// 创建标签引擎。
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: GlyphCache::new(),
            engine: TextEngine::new(),
            raster_scale: 1.0,
            prewarmed: None,
        }
    }

    /// 按 `logical_size × scale` 光栅化，并以逻辑单位放置四边形。
    pub fn set_raster_scale(&mut self, scale: f32) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        self.raster_scale = scale.clamp(0.5, 4.0);
    }

    /// 是否按整像素网格光栅化字形。
    pub fn set_snap_phases(&mut self, snap: bool) {
        self.cache.snap_phases = snap;
    }

    /// 字形光栅化所采用的缩放比例。
    #[must_use]
    pub fn raster_scale(&self) -> f32 {
        self.raster_scale
    }

    /// 使运行段在 `rect` 内居中的原点。
    ///
    /// 垂直居中采用字体的升部与降部而非运行段的墨水框，使同字号的所有标签
    /// 落在同一条基线上。水平居中采用步进宽度，即字体自身认定的运行段宽度。
    ///
    ///
    #[must_use]
    pub fn center_in_rect(metrics: &TextMetrics, rect: Rect) -> Vec2 {
        let x = rect.origin.x + (rect.size.x - metrics.width) * 0.5;
        // 行框的视觉中线：基线位于框中心上方半个行框高度处。
        //
        let middle = (metrics.ascent - metrics.descent) * 0.5;
        let target_baseline = rect.origin.y + rect.size.y * 0.5 + middle;
        Vec2::new(x, target_baseline - metrics.first_baseline)
    }

    /// 运行段在 `size` 字号下的度量信息（逻辑单位）。
    pub fn metrics(&mut self, text: &str, size: f32) -> TextMetrics {
        let scale = self.raster_scale;
        let shaped = self.engine.shape(text, &TextStyle::sized(size * scale));
        metrics_of(&shaped, scale)
    }

    /// 在显示之前预先光栅化动态标签可能产生的所有字形。
    ///
    /// 图集在启动时一次性上传。若点击按钮使 `"Clicked: 0"` 变为 `"Clicked: 1"`
    /// 而 `'1'` 从未被塑形过，四边形就会采样空的图集页并渲染成空白方块。
    /// 预先塑形整套字符表可使点击保持零纹理上传。
    ///
    ///
    pub fn prewarm(&mut self, text: &str, size: f32) {
        let scale = self.raster_scale;
        if self.prewarmed == Some((size, scale)) {
            return;
        }
        let style = TextStyle::sized(size * scale);
        let shaped = self.engine.shape(text, &style);
        let _ = self
            .cache
            .quads_for(&mut self.engine, &shaped, Vec2::ZERO, 400, false);
        self.prewarmed = Some((size, scale));
    }

    /// 布局标签并返回其字形四边形。
    ///
    /// `origin` 为文本**行框**的左上角（逻辑单位）：首行基线位于 `origin.y + ascent`，
    /// 字形悬挂在其上方。
    ///
    ///
    /// 运行段刻意**不**按墨水对齐。按墨水框对齐会把每个字符串移到不同的基线上，
    /// 因为墨水顶部取决于字符串自身最高的字形——这正是导致一排按钮的标签
    /// 处于略微不同高度的原因。
    ///
    ///
    /// 原点在生成四边形**之前**传入字形缓存，使每个字形的亚像素相位均由其
    /// 在屏幕上的真实位置推导。若从相对运行段的位置推导相位，
    /// 每个字形最多会偏移一个像素。
    ///
    pub fn layout(&mut self, text: &str, origin: Vec2, size: f32) -> Vec<GlyphQuad> {
        self.layout_measured(text, origin, size).0
    }

    /// 布局标签并报告其度量信息。
    pub fn layout_measured(
        &mut self,
        text: &str,
        origin: Vec2,
        size: f32,
    ) -> (Vec<GlyphQuad>, TextMetrics) {
        let scale = self.raster_scale;
        let style = TextStyle::sized(size * scale);
        let shaped = self.engine.shape(text, &style);
        let metrics = metrics_of(&shaped, scale);
        let mut quads = self
            .cache
            .quads_for(&mut self.engine, &shaped, origin * scale, 400, false);
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        (quads, metrics)
    }

    /// 测量单行运行段，不发射字形。
    #[must_use]
    pub fn measure(&mut self, text: &str, size: f32) -> Vec2 {
        let scale = self.raster_scale;
        self.engine.measure(text, &TextStyle::sized(size * scale)) / scale
    }

    /// 逐行布局自动换行段落，每次塑形一行。
    ///
    /// 换行位置来自引擎的换行缓存，因此调整卡片尺寸仅重算断行位置，
    /// 绝不重新塑形字形。
    pub fn layout_wrapped(
        &mut self,
        text: &str,
        origin: Vec2,
        size: f32,
        max_width: f32,
    ) -> Vec<GlyphQuad> {
        let scale = self.raster_scale;
        let style = TextStyle::sized(size * scale);
        let shaped = self
            .engine
            .shape_wrapped(text, &style, (max_width * scale).max(1.0));
        let mut quads = self
            .cache
            .quads_for(&mut self.engine, &shaped, origin * scale, 400, false);
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        quads
    }

    /// 使用显式字重、倾斜与字体族布局标签。
    ///
    /// [`Self::layout`] 固定为常规正体。字体检查器会调节整条轴，且光栅化字重
    /// 必须与塑形字重一致——更粗的光栅配常规步进正是导致小标签显得拥挤的原因。
    ///
    ///
    pub fn layout_styled(&mut self, text: &str, origin: Vec2, style: &TextStyle) -> Vec<GlyphQuad> {
        self.layout_styled_measured(text, origin, style).0
    }

    /// 布局带样式的运行段并报告其逻辑度量信息。
    pub fn layout_styled_measured(
        &mut self,
        text: &str,
        origin: Vec2,
        style: &TextStyle,
    ) -> (Vec<GlyphQuad>, TextMetrics) {
        let scale = self.raster_scale;
        let mut style = style.clone();
        style.font_size *= scale;
        let shaped = self.engine.shape(text, &style);
        let metrics = metrics_of(&shaped, scale);
        let mut quads = self.cache.quads_for(
            &mut self.engine,
            &shaped,
            origin * scale,
            style.weight,
            style.italic,
        );
        if (scale - 1.0).abs() > 1e-6 {
            for quad in &mut quads {
                quad.rect.origin.x /= scale;
                quad.rect.origin.y /= scale;
                quad.rect.size.x /= scale;
                quad.rect.size.y /= scale;
            }
        }
        (quads, metrics)
    }

    /// 测量带样式的运行段，不发射字形。
    #[must_use]
    pub fn measure_styled(&mut self, text: &str, style: &TextStyle) -> Vec2 {
        let scale = self.raster_scale;
        let mut style = style.clone();
        style.font_size *= scale;
        self.engine.measure(text, &style) / scale
    }

    /// 带样式运行段的逻辑度量信息。
    pub fn metrics_styled(&mut self, text: &str, style: &TextStyle) -> TextMetrics {
        let scale = self.raster_scale;
        let mut style = style.clone();
        style.font_size *= scale;
        let shaped = self.engine.shape(text, &style);
        metrics_of(&shaped, scale)
    }

    /// 实际塑形 `text` 所使用的系统字体族。
    pub fn resolved_families(&mut self, text: &str, style: &TextStyle) -> Vec<String> {
        let scale = self.raster_scale;
        let mut style = style.clone();
        style.font_size *= scale;
        self.engine.resolved_families(text, &style)
    }
}

/// 已塑形运行段的逻辑度量信息。
fn metrics_of(shaped: &ShapedText, scale: f32) -> TextMetrics {
    TextMetrics {
        width: shaped.width / scale,
        height: shaped.height / scale,
        ascent: shaped.ascent / scale,
        descent: shaped.descent / scale,
        first_baseline: shaped.first_baseline / scale,
        line_count: shaped.line_count,
    }
}

impl Default for Labels {
    fn default() -> Self {
        Self::new()
    }
}

/// 按钮在画布坐标中的矩形。
#[must_use]
pub fn button_rect() -> Rect {
    Rect::new(120.0, 468.0, 200.0, 44.0)
}

/// 当调用方不持有 `UiWorld`（离屏 `--shot` 路径与视觉测试）时，
/// [`build`] 赋予按钮的占位标识。
pub const DEFAULT_BUTTON_NODE: u64 = 2;

/// 展厅上报的语义树根节点。
const A11Y_ROOT: u64 = 1;

/// 动态按钮标签可能包含的字符。预先塑形以使点击永不扩容图集。
///
const BUTTON_LABEL_ALPHABET: &str = "Clicked: 0123456789";

/// 构建处于空闲状态的完整展厅。
///
/// 返回场景以及调用方仍需上传到 GPU 图集的字形四边形。
///
pub fn build(
    labels: &mut Labels,
    palette: &Palette,
    viewport: (f32, f32),
) -> (Gallery, Vec<GlyphQuad>) {
    let button = ButtonDesc::new(button_rect(), "Clicked: 0");
    let (scene, glyphs, _, _) = build_interactive(
        labels,
        palette,
        viewport,
        &button,
        DEFAULT_BUTTON_NODE,
        ButtonVisualState::default(),
        0,
    );
    (scene, glyphs)
}

/// 构建按钮处于 `visual` 状态的展厅。
///
/// 返回场景、按钮标签的字形四边形（已置于场景空间）、指针路由器应持有的命中区域，
/// 以及语义树。该签名是迈向平台层将消费的 `(Scene, HitRegions, Semantics)` 三元组的第一步。
///
///
pub fn build_interactive(
    labels: &mut Labels,
    palette: &Palette,
    viewport: (f32, f32),
    button: &ButtonDesc,
    node: u64,
    visual: ButtonVisualState,
    click_count: u32,
) -> (Gallery, Vec<GlyphQuad>, Vec<HitRegion>, Semantics) {
    let mut scene = Gallery::new();
    scene.camera = Camera::ui(viewport, 1.0);

    // `local_pos` 是区块 [0, 0] 内的绝对位置，对于 UI 图元而言*就是*屏幕位置：
    // 着色器会自行减去相机偏移（`chunk_base = -camera_local`）。
    //
    let at = |x: f32, y: f32| (x, y);

    // ---- 页面背景 ------------------------------------------------------------
    let (bx, by) = at(0.0, 0.0);
    scene.rect(
        bx,
        by,
        viewport.0,
        viewport.1,
        0.0,
        palette.background,
        palette.background,
        0.0,
        0.0,
        0,
    );

    // ---- 标题 ----------------------------------------------------------------
    let title = labels.layout(
        "Unbound UI — 组件展台",
        Vec2::new(at(40.0, 32.0).0, at(40.0, 32.0).1),
        28.0,
    );
    for quad in &title {
        scene.glyph(quad, palette.text, 100);
    }

    // ---- 图元网格 ------------------------------------------------------------
    let swatches = [
        ("Primary", palette.primary),
        ("Secondary", palette.secondary),
        ("Success", palette.success),
        ("Warning", palette.warning),
        ("Danger", palette.danger),
    ];

    for (index, (name, token)) in swatches.iter().enumerate() {
        let x = 40.0 + index as f32 * 150.0;
        // 反向 Z（Reverse-Z）：深度越大越近且越后绘制，
        // 因此色板位于页面背景之上，其标签位于色板之上。
        //
        let z = 200 + index as u16;
        let (rx, ry) = at(x, 100.0);
        // 解析阴影是图元自身的属性：着色器根据 `shadow_blur` 推导，
        // 无需额外的绘制调用。
        scene.rect(
            rx,
            ry,
            120.0,
            64.0,
            12.0,
            *token,
            palette.border,
            1.0,
            14.0,
            z + 1,
        );
        let label = labels.layout(
            name,
            Vec2::new(at(x + 10.0, 174.0).0, at(x + 10.0, 174.0).1),
            13.0,
        );
        for quad in &label {
            scene.glyph(quad, palette.text_muted, z + 2);
        }
    }

    // ---- 带边框和阴影的卡片 --------------------------------------------------
    let (cx, cy) = at(40.0, 230.0);
    scene.rect(
        cx,
        cy,
        360.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        18.0,
        301,
    );
    let card_title = labels.layout(
        "Card / 圆角 + 边框 + 解析阴影",
        Vec2::new(at(60.0, 250.0).0, at(60.0, 250.0).1),
        16.0,
    );
    for quad in &card_title {
        scene.glyph(quad, palette.text, 302);
    }
    let card_body = labels.layout(
        "Analytic shadow (erf), zero offscreen passes.",
        Vec2::new(at(60.0, 278.0).0, at(60.0, 278.0).1),
        13.0,
    );
    for quad in &card_body {
        scene.glyph(quad, palette.text_muted, 302);
    }

    // ---- 描边（焦点环）------------------------------------------------------
    let (sx, sy) = at(430.0, 230.0);
    scene.rect(
        sx,
        sy,
        160.0,
        150.0,
        16.0,
        palette.surface,
        palette.surface,
        0.0,
        0.0,
        400,
    );
    scene.stroke(sx, sy, 160.0, 150.0, 16.0, palette.primary, 401);
    let stroke_label = labels.layout(
        "Stroke / 焦点环",
        Vec2::new(at(sx + 12.0, sy + 120.0).0, at(sx + 12.0, sy + 120.0).1),
        13.0,
    );
    for quad in &stroke_label {
        scene.glyph(quad, palette.text_muted, 402);
    }

    // ---- 曲线 ----------------------------------------------------------------
    let (gx, gy) = at(620.0, 230.0);
    scene.rect(
        gx,
        gy,
        300.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        0.0,
        500,
    );
    for (index, color) in [palette.primary, palette.secondary, palette.success]
        .iter()
        .enumerate()
    {
        let f = index as f32;
        scene.curve(
            [gx + 20.0, gy + 30.0 + f * 30.0],
            [gx + 150.0, gy + 110.0 - f * 20.0],
            [gx + 280.0, gy + 30.0 + f * 30.0],
            3.0,
            *color,
            501 + index as u16,
        );
    }
    let curve_label = labels.layout(
        "Curves / 二次贝塞尔解析 SDF",
        Vec2::new(at(gx + 14.0, gy + 120.0).0, at(gx + 14.0, gy + 120.0).1),
        13.0,
    );
    for quad in &curve_label {
        scene.glyph(quad, palette.text_muted, 505);
    }

    // ---- 交互式卡片 ----------------------------------------------------------
    scene.rect(
        40.0,
        410.0,
        360.0,
        150.0,
        16.0,
        palette.surface,
        palette.border,
        1.0,
        18.0,
        700,
    );
    let interactive_title =
        labels.layout("Button / 第一个可交互控件", Vec2::new(60.0, 430.0), 16.0);
    for quad in &interactive_title {
        scene.glyph(quad, palette.text, 701);
    }
    labels.prewarm(BUTTON_LABEL_ALPHABET, button.font_size);
    let mut glyphs = labels.layout(&button.label, Vec2::ZERO, button.font_size);
    let artifacts = button::emit_button(node, button, visual, &mut glyphs);
    let base = scene.payloads.len() as u32;
    let mut primitives = artifacts.primitives;
    for prim in &mut primitives {
        if matches!(prim.prim_type(), PrimType::Glyph | PrimType::GlyphColor) {
            prim.extra_payload_idx += base;
        }
    }
    scene.primitives.extend(primitives);
    scene.payloads.extend(artifacts.payloads);

    let button_hint = labels.layout(
        "hover / press / click · Tab 聚焦 · Space 激活",
        Vec2::new(60.0, 524.0),
        13.0,
    );
    for quad in &button_hint {
        scene.glyph(quad, palette.text_muted, 701);
    }

    // ---- 主题提示 ------------------------------------------------------------
    let hint = labels.layout(
        "T / 切换主题    Space / 主题    滚轮 / 缩放    拖拽 / 平移",
        Vec2::new(at(40.0, viewport.1 - 60.0).0, at(40.0, viewport.1 - 60.0).1),
        14.0,
    );
    for quad in &hint {
        scene.glyph(quad, palette.text_muted, 600);
    }

    // ---- 语义 ----------------------------------------------------------------
    let mut semantics = Semantics::new(A11Y_ROOT);
    semantics.upsert(
        A11Y_ROOT,
        A11yNode::new(
            Role::Window,
            "Unbound UI — 组件展台",
            Rect::new(0.0, 0.0, viewport.0, viewport.1),
        ),
    );
    let mut button_node = artifacts.a11y.unwrap_or_default();
    button_node.value = click_count.to_string();
    semantics.upsert(node, button_node);
    semantics.set_children(A11Y_ROOT, &[node]);

    // 字形已纳入场景中；它们产生的图集更新由调用方排空并上传至 GPU。
    //
    (
        scene,
        glyphs,
        artifacts.hit.into_iter().collect(),
        semantics,
    )
}

/// 供五项校验器使用的按钮无头几何帧。
///
/// 紧邻发射器存放，使指针命中的矩形、帧校验的矩形与图元绘制的矩形永不漂移。
///
#[must_use]
pub fn button_frame(viewport: (f32, f32), button: &ButtonDesc, glyphs: &[GlyphQuad]) -> Frame {
    let viewport = Rect::new(0.0, 0.0, viewport.0, viewport.1);
    let mut frame = FrameBuilder::new(viewport);
    frame.open(NodeKind::Container, viewport, "Root");
    frame.open(
        NodeKind::Container,
        Rect::new(40.0, 410.0, 360.0, 150.0),
        "ButtonCard",
    );
    frame.open(
        NodeKind::Interactive {
            min_size: NodeKind::DEFAULT_MIN_SIZE,
            occluded: false,
        },
        button.rect,
        "Button",
    );
    frame.set_clip(button.rect);
    if let Some(ink) = button::ink_bounds(glyphs) {
        frame.open(
            NodeKind::Text {
                ink,
                ellipsis: false,
            },
            ink,
            "Label",
        );
        frame.close();
    }
    frame.close();
    frame.close();
    frame.close();
    frame.build()
}

/// 场景的包围矩形，供布局测试使用。
#[must_use]
pub fn scene_bounds(scene: &Gallery) -> Rect {
    let mut bounds = Rect::ZERO;
    for prim in &scene.primitives {
        let rect = Rect::new(
            prim.local_pos[0],
            prim.local_pos[1],
            prim.size[0],
            prim.size[1],
        );
        bounds = bounds.union(&rect);
    }
    bounds
}
