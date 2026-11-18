//! 后景计划：把模式选择、平台能力与屏幕几何收敛成一份纯值。
//!
//! 宿主不再各自推导「该用哪条路径 / 清屏 alpha 多少 / 纹理多大 / 取哪块 UV」，
//! 而是调用 [`BackdropPlan::derive`] 一次算清，再让启动、运行期切换与自绘生成
//! 三处消费同一份结果。该模块不做任何系统调用，输入必须由调用方提供，因此
//! 可以在单元测试里穷举全部组合与各种几何。
//!
//! 组合解析本身在 [`crate::selection`]：这里只负责把解析结论展开成计划。

use crate::mica::{DEFAULT_MAX_WIDTH, DEFAULT_SIGMA};
use crate::selection::{
    resolve_combination, BackdropCapabilities, BackdropSelection, BackdropSource, FallbackReason,
    MaterialKind, UnavailableReason,
};

/// 屏幕坐标系中的物理像素矩形。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    /// 左上角物理 X 坐标。
    pub x: i32,
    /// 左上角物理 Y 坐标。
    pub y: i32,
    /// 物理宽度。
    pub width: u32,
    /// 物理高度。
    pub height: u32,
}

impl PixelRect {
    /// 构造一个物理矩形。
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 是否具有可用的面积。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 右边界（物理像素，不含）。
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    /// 下边界（物理像素，不含）。
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }
}

/// 归一化 UV 子矩形，取值 `0..1`。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvRect {
    /// 左边界。
    pub x: f32,
    /// 上边界。
    pub y: f32,
    /// 宽度。
    pub width: f32,
    /// 高度。
    pub height: f32,
}

impl UvRect {
    /// 整张纹理。
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// 构造一个归一化子矩形。
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// 表面合成方式。
///
/// 与用户选择和后景路径无关，只由平台能力决定：原生材质需要预乘 alpha，
/// 不支持时只能用不透明表面。同一窗口生命周期内必须保持不变。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BackdropCompositeAlpha {
    /// 不透明表面：清屏 alpha 为 1，窗口永远遮住桌面。
    #[default]
    Opaque,
    /// 预乘 alpha 表面：清屏 alpha 可为 0，让 DWM 材质透出。
    PreMultiplied,
}

/// 需要向 DWM 请求的系统材质。
///
/// 这里只声明「请求哪一档」，不携带平台的原始取值：平台层负责把请求翻译成
/// 系统调用需要的整数。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum DwmMaterial {
    /// 不请求任何材质（关闭后景或走自绘路径）。
    #[default]
    None,
    /// 跟随系统默认策略。
    System,
    /// 标准云母材质。
    Mica,
    /// 云母变体材质。
    MicaAlt,
    /// 亚克力材质。
    Acrylic,
    /// 沉浸式后景（实验档）。
    ///
    /// 该档的系统取值没有出现在公开文档里，平台层的「写入后回读」探测也确认不了
    /// 真实取值（详见 `unbound_platform::win32_frame::probe_immersive_backdrop`），因此没有任何
    /// 材质种类会请求它：它留在枚举里只是为将来实测出真实取值后留一个落点，
    /// 绝不携带猜测出来的数字。
    Immersive,
}

/// 材质实际走哪一条系统通道。
///
/// DWM 材质通道与 accent policy 通道是两条互斥的写入路径：同一个窗口同时只能
/// 走其中一条，宿主必须按实际生效的通道二选一，绝不能两条都下发。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum MaterialChannel {
    /// 不请求任何系统材质。
    #[default]
    None,
    /// 走 `DWMWA_SYSTEMBACKDROP_TYPE` 系统材质通道。
    Dwm(DwmMaterial),
    /// 走 `SetWindowCompositionAttribute` 的 accent policy 通道。
    ///
    /// 该通道只承载亚克力自定义色调，参数是一个 ABGR 色调值；它不经过 DWM，
    /// 因此不能表达成 [`DwmMaterial`] 的取值。
    AccentPolicy,
}

/// 一行材质映射：某个材质种类走哪条系统通道，以及（走 DWM 时）请求哪一档材质。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MaterialRequest {
    /// 该行的材质种类。
    pub kind: MaterialKind,
    /// 该材质种类使用的系统通道。
    pub channel: MaterialChannel,
}

/// 材质种类到系统通道的显式映射表。
///
/// 表格是写死的一行一档，而不是从枚举推导出来的：将来出现不走系统材质通道
/// 的材质时，只需新增一行，不必改动转换逻辑。
pub const MATERIAL_REQUEST_TABLE: [MaterialRequest; MaterialKind::ALL.len()] = [
    MaterialRequest {
        kind: MaterialKind::None,
        channel: MaterialChannel::None,
    },
    MaterialRequest {
        kind: MaterialKind::FollowSystem,
        channel: MaterialChannel::Dwm(DwmMaterial::System),
    },
    MaterialRequest {
        kind: MaterialKind::Mica,
        channel: MaterialChannel::Dwm(DwmMaterial::Mica),
    },
    MaterialRequest {
        kind: MaterialKind::MicaAlt,
        channel: MaterialChannel::Dwm(DwmMaterial::MicaAlt),
    },
    MaterialRequest {
        kind: MaterialKind::Acrylic,
        channel: MaterialChannel::Dwm(DwmMaterial::Acrylic),
    },
    // 沉浸式后景的系统取值没有公开文档依据，平台层的写入回读探测也只能看到
    // 「原样回显」，无法证明系统真的接受某个未公开取值。既然确认不了，就不写死
    // 任何数字：该档当前不走系统材质通道，窗口必须保持不透明，界面另给不可用原因。
    MaterialRequest {
        kind: MaterialKind::Immersive,
        channel: MaterialChannel::None,
    },
    // 亚克力自定义色调走独立的 accent policy 通道，不占用 DWM 材质取值。
    MaterialRequest {
        kind: MaterialKind::AcrylicTint,
        channel: MaterialChannel::AccentPolicy,
    },
];

/// 按映射表查出某个材质种类使用的系统通道。
#[must_use]
pub fn material_channel(material: MaterialKind) -> MaterialChannel {
    MATERIAL_REQUEST_TABLE
        .iter()
        .find(|row| row.kind == material)
        .map_or(MaterialChannel::None, |row| row.channel)
}

/// 按映射表查出某个材质种类的 DWM 材质请求。
///
/// 只有走 DWM 通道的材质种类返回 `Some`；走 accent policy 通道的材质返回
/// `None`，它的系统请求通过 [`BackdropPlan::accent_tint`] 表达。
#[must_use]
pub fn system_material_request(material: MaterialKind) -> Option<DwmMaterial> {
    match material_channel(material) {
        MaterialChannel::Dwm(material) => Some(material),
        MaterialChannel::None | MaterialChannel::AccentPolicy => None,
    }
}

/// 默认的亚克力自定义色调（ABGR，中性灰，约 80% 不透明）。
///
/// 色调是我们自己的选择，不属于系统取值；alpha 分量参与 accent policy 的
/// `GradientColor`，让亚克力保持半透明。
pub const DEFAULT_ACRYLIC_TINT_ARGB: u32 = 0xCC1F1F1F;

/// 一个亚克力自定义色调预设。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AcrylicTintPreset {
    /// 预设名称（英文，用于界面展示）。
    pub name: &'static str,
    /// 预设色调（ABGR）。
    pub argb: u32,
}

/// 少量现成的亚克力自定义色调预设。
pub const ACRYLIC_TINT_PRESETS: [AcrylicTintPreset; 3] = [
    AcrylicTintPreset {
        name: "Neutral",
        argb: DEFAULT_ACRYLIC_TINT_ARGB,
    },
    AcrylicTintPreset {
        name: "Light",
        argb: 0xCCF0F0F0,
    },
    // 接近 Windows 默认强调蓝（RGB 0x0078D7），ABGR 分量顺序相反。
    AcrylicTintPreset {
        name: "Accent",
        argb: 0xD0D77800,
    },
];

/// 自绘后景的纹理规格。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WallpaperTexture {
    /// 纹理覆盖区域的物理宽。
    pub width: u32,
    /// 纹理覆盖区域的物理高。
    pub height: u32,
    /// 主屏物理宽，用于把窗口映射进壁纸空间。
    pub screen_width: u32,
    /// 主屏物理高，用于把窗口映射进壁纸空间。
    pub screen_height: u32,
    /// 系统 DPI 缩放；纳入纹理失效条件。
    pub dpi_scale: f32,
    /// 纹理最大宽度（0 视为默认值）。
    pub max_width: u32,
    /// 磨砂 sigma，单位是屏幕物理像素。
    pub sigma: f32,
    /// 是否使用深色主题色调。
    pub dark: bool,
    /// 实际生效的材质种类；它选择色调配方，并纳入纹理失效条件。
    pub material: MaterialKind,
}

impl WallpaperTexture {
    /// 纹理最大宽度，已把 0 归一化为默认值。
    #[must_use]
    pub const fn effective_max_width(&self) -> u32 {
        if self.max_width == 0 {
            DEFAULT_MAX_WIDTH
        } else {
            self.max_width
        }
    }

    /// 磨砂 sigma（屏幕物理像素），已把负值归一化为 0。
    #[must_use]
    pub fn effective_sigma(&self) -> f32 {
        self.sigma.max(0.0)
    }
}

/// 后景计划的输入。全部是值，不含系统查询。
///
/// 平台能力按「逐档门槛」所需的零件给出，而不是一个笼统的布尔量：
/// 每档材质的门槛不同，必须由调用方提供构建号与表面合成能力，判定才能逐档进行。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BackdropInputs {
    /// 用户选择：材质种类与来源两个正交维度。
    pub selection: BackdropSelection,
    /// 系统构建号；`None` 表示非系统环境或探测失败。
    pub windows_build: Option<u32>,
    /// 图形表面是否支持预乘 alpha 合成。
    pub premultiplied_alpha_supported: bool,
    /// 运行时能否解析出 `SetWindowCompositionAttribute` 并通过冒烟探测。
    pub accent_policy_export: bool,
    /// 亚克力自定义色调（ABGR）；只在自定义色调子档走原生路径时使用。
    pub accent_tint_argb: u32,
    /// 客户区屏幕物理矩形；未知为 `None`。
    pub window_screen_rect: Option<PixelRect>,
    /// 主屏物理矩形；未知为 `None`。
    pub primary_screen_rect: Option<PixelRect>,
    /// 系统 DPI 缩放；未知或非法时按 `1.0` 处理。
    pub primary_screen_scale: f32,
    /// 是否深色主题。
    pub dark: bool,
    /// 自绘纹理是否已经就绪（用于根背景透明性）。
    pub self_drawn_ready: bool,
}

/// 后景计划：启动、切换与生成共同消费的唯一结果。
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropPlan {
    /// 实际生效的材质种类；`None` 表示完全没有后景。
    pub material_kind: MaterialKind,
    /// 实际生效的来源。
    pub source: BackdropSource,
    /// 请求被降级的原因；`None` 表示原样满足。
    pub fallback: Option<FallbackReason>,
    /// 机器能力不足的原因；`None` 表示机器能力没问题。
    pub unavailable: Option<UnavailableReason>,
    /// 固定合成方式，只由平台能力决定。
    pub composition: BackdropCompositeAlpha,
    /// 清屏 alpha：真正请求到系统材质时 0，其余路径 1。
    ///
    /// 拿不到系统材质请求的路径（含实验档与所有自绘路径）一律保持 1，
    /// 否则窗口会变成「透明却没有材质」的空壳。
    pub clear_alpha: f32,
    /// 根背景是否应为透明，交给后景透出。
    pub root_background_transparent: bool,
    /// 需要请求的 DWM 材质。
    pub material: DwmMaterial,
    /// 需要下发的 accent policy 色调（ABGR）；非该通道路径时为 `None`。
    ///
    /// 与 [`Self::material`] 互斥：同一份计划最多只有一条系统通道带值。
    pub accent_tint: Option<u32>,
    /// 自绘纹理规格；非自绘路径或几何未知时为 `None`。
    pub texture: Option<WallpaperTexture>,
    /// 自绘纹理的归一化采样区域。
    pub uv: UvRect,
}

impl Default for BackdropPlan {
    fn default() -> Self {
        Self {
            material_kind: MaterialKind::None,
            source: BackdropSource::Auto,
            fallback: None,
            unavailable: None,
            composition: BackdropCompositeAlpha::Opaque,
            clear_alpha: 1.0,
            root_background_transparent: false,
            material: DwmMaterial::None,
            accent_tint: None,
            texture: None,
            uv: UvRect::FULL,
        }
    }
}

impl BackdropPlan {
    /// 由输入推导完整计划。
    ///
    /// 先用组合解析接缝算出实际生效的组合，再把结论展开成计划：
    ///
    /// - 系统通道走显式映射表，且只在实际来源为原生时才下发；DWM 材质与
    ///   accent policy 色调互斥，计划里最多只有一边带值；
    /// - 清屏透明度只在真的拿到系统请求时才变成 0，其余一律 1；
    /// - 根背景透明性由清屏 alpha 与自绘纹理就绪状态共同决定；
    /// - 纹理只在实际来源为自绘时生成；
    /// - 合成方式只由平台基线能力决定，与材质种类、来源无关——交换链在启动时
    ///   一次性建好，切换绝不能重建它。
    #[must_use]
    pub fn derive(inputs: BackdropInputs) -> Self {
        let caps = BackdropCapabilities {
            windows_build: inputs.windows_build,
            premultiplied_alpha_supported: inputs.premultiplied_alpha_supported,
            accent_policy_export: inputs.accent_policy_export,
        };
        let combination = resolve_combination(inputs.selection, &caps);
        // 材质种类为「无」时来源轴折叠：没有任何后景可言。
        let has_backdrop = combination.material != MaterialKind::None;
        let native = has_backdrop && combination.source == BackdropSource::Native;
        let self_drawn = has_backdrop && combination.source == BackdropSource::SelfDrawn;
        let composition = if caps.baseline_native_available() {
            BackdropCompositeAlpha::PreMultiplied
        } else {
            BackdropCompositeAlpha::Opaque
        };
        let texture = if self_drawn {
            wallpaper_texture(inputs, combination.material)
        } else {
            None
        };
        // 只有原生来源才可能拿到系统请求。DWM 通道与 accent policy 通道互斥：
        // 一份计划最多只会填其中一边。
        let channel = if native {
            material_channel(combination.material)
        } else {
            MaterialChannel::None
        };
        let request = match channel {
            MaterialChannel::Dwm(material) => Some(material),
            MaterialChannel::None | MaterialChannel::AccentPolicy => None,
        };
        // accent policy 通道同样由系统合成器绘制，清屏也必须全透明，否则亚克力
        // 会被盖住。这里按「有没有系统请求」判定，而不是只看 DWM 材质。
        let has_system_request = matches!(
            channel,
            MaterialChannel::Dwm(_) | MaterialChannel::AccentPolicy
        );
        let material = request.unwrap_or(DwmMaterial::None);
        let accent_tint = if channel == MaterialChannel::AccentPolicy {
            Some(inputs.accent_tint_argb)
        } else {
            None
        };
        let clear_alpha = if has_system_request { 0.0 } else { 1.0 };
        Self {
            material_kind: combination.material,
            source: combination.source,
            fallback: combination.fallback,
            unavailable: combination.unavailable,
            composition,
            clear_alpha,
            root_background_transparent: clear_alpha == 0.0
                || (self_drawn && inputs.self_drawn_ready),
            material,
            accent_tint,
            texture,
            // 纹理覆盖整块主屏，窗口只取自己所占的那一块。
            uv: sample_uv(inputs.window_screen_rect, inputs.primary_screen_rect),
        }
    }
}

/// 构造自绘纹理规格。
///
/// `material` 必须是组合解析出的**实际生效**材质，而不是用户选择值：回退到
/// 标准云母自绘时它就是 [`MaterialKind::Mica`]，云母变体则保留自己的种类，
/// 纹理因此带上正确的色调配方。
fn wallpaper_texture(inputs: BackdropInputs, material: MaterialKind) -> Option<WallpaperTexture> {
    let window = inputs.window_screen_rect?;
    let screen = inputs.primary_screen_rect?;
    if window.is_empty() || screen.is_empty() {
        return None;
    }
    Some(WallpaperTexture {
        width: screen.width.max(1),
        height: screen.height.max(1),
        screen_width: screen.width.max(1),
        screen_height: screen.height.max(1),
        dpi_scale: if inputs.primary_screen_scale.is_finite() && inputs.primary_screen_scale > 0.0 {
            inputs.primary_screen_scale
        } else {
            1.0
        },
        max_width: DEFAULT_MAX_WIDTH,
        sigma: DEFAULT_SIGMA,
        dark: inputs.dark,
        material,
    })
}

/// 把窗口物理矩形映射为主屏纹理的归一化采样区域。
///
/// 部分越界时按主屏边界钳制；完全无交集或几何未知时退回整张纹理，
/// 保证采样区域始终有效。主屏原点参与计算，副屏坐标不会被误当成 `0..1`。
#[must_use]
pub fn sample_uv(window: Option<PixelRect>, screen: Option<PixelRect>) -> UvRect {
    let (Some(window), Some(screen)) = (window, screen) else {
        return UvRect::FULL;
    };
    if window.is_empty() || screen.is_empty() {
        return UvRect::FULL;
    }
    let left = window.x.max(screen.x);
    let top = window.y.max(screen.y);
    let right = window.right().min(screen.right());
    let bottom = window.bottom().min(screen.bottom());
    if right <= left || bottom <= top {
        return UvRect::FULL;
    }
    UvRect::new(
        (left - screen.x) as f32 / screen.width as f32,
        (top - screen.y) as f32 / screen.height as f32,
        (right - left) as f32 / screen.width as f32,
        (bottom - top) as f32 / screen.height as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: PixelRect = PixelRect::new(0, 0, 1920, 1080);

    /// 平台能力矩阵：构建号未知、门槛下沿、门槛上沿与实验档门槛两侧。
    const CAPABILITIES: [(Option<u32>, bool); 10] = [
        (None, true),
        (None, false),
        (Some(22620), true),
        (Some(22620), false),
        (Some(22621), true),
        (Some(22621), false),
        (Some(26100), true),
        (Some(26100), false),
        (Some(26101), true),
        (Some(26101), false),
    ];

    fn inputs(
        selection: BackdropSelection,
        build: Option<u32>,
        premultiplied: bool,
    ) -> BackdropInputs {
        BackdropInputs {
            selection,
            windows_build: build,
            premultiplied_alpha_supported: premultiplied,
            accent_policy_export: true,
            accent_tint_argb: DEFAULT_ACRYLIC_TINT_ARGB,
            window_screen_rect: Some(PixelRect::new(480, 270, 960, 540)),
            primary_screen_rect: Some(SCREEN),
            primary_screen_scale: 1.5,
            dark: false,
            self_drawn_ready: false,
        }
    }

    /// 平台基线能力：能否承载原生后景（只取决于构建号与预乘合成支持）。
    fn native_baseline(build: Option<u32>, premultiplied: bool) -> bool {
        BackdropCapabilities {
            windows_build: build,
            premultiplied_alpha_supported: premultiplied,
            accent_policy_export: false,
        }
        .baseline_native_available()
    }

    /// 手写的“该材质种类请求哪一档系统材质”期望，独立于映射表实现。
    ///
    /// 沉浸式后景的系统取值未被探测确认，因此没有请求；亚克力自定义色调走
    /// accent policy 通道，同样不产生 DWM 请求。
    fn expected_system_request(material: MaterialKind) -> Option<DwmMaterial> {
        match material {
            MaterialKind::None => None,
            MaterialKind::FollowSystem => Some(DwmMaterial::System),
            MaterialKind::Mica => Some(DwmMaterial::Mica),
            MaterialKind::MicaAlt => Some(DwmMaterial::MicaAlt),
            MaterialKind::Acrylic => Some(DwmMaterial::Acrylic),
            MaterialKind::Immersive => None,
            MaterialKind::AcrylicTint => None,
        }
    }

    #[test]
    fn the_default_choice_keeps_its_original_effects() {
        // 标准云母 + 自动来源，原生可用：走原生路径，清屏全透明，请求标准云母，不生成纹理。
        let native = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto),
            Some(26100),
            true,
        ));
        assert_eq!(native.material_kind, MaterialKind::Mica);
        assert_eq!(native.source, BackdropSource::Native);
        assert_eq!(native.clear_alpha, 0.0);
        assert_eq!(native.material, DwmMaterial::Mica);
        assert!(native.root_background_transparent);
        assert!(native.texture.is_none());
        assert_eq!(native.fallback, None);
        assert_eq!(native.unavailable, None);

        // 显式原生但机器承载不了：回退到自绘，材质种类不变。
        let fallback = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
            Some(19045),
            true,
        ));
        assert_eq!(fallback.material_kind, MaterialKind::Mica);
        assert_eq!(fallback.source, BackdropSource::SelfDrawn);
        assert_eq!(fallback.clear_alpha, 1.0);
        assert_eq!(fallback.material, DwmMaterial::None);
        assert!(!fallback.root_background_transparent);
        assert!(fallback.texture.is_some());
        assert_eq!(fallback.fallback, None);
        assert_eq!(
            fallback.unavailable,
            Some(UnavailableReason::BuildTooLow { required: 22621 }),
            "an explicit native choice must report the capability shortfall"
        );

        // 显式自绘：任何机器上都走自绘。
        let self_drawn = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        ));
        assert_eq!(self_drawn.source, BackdropSource::SelfDrawn);
        assert_eq!(self_drawn.clear_alpha, 1.0);
        assert_eq!(self_drawn.material, DwmMaterial::None);
        assert!(self_drawn.texture.is_some());
        assert_eq!(self_drawn.fallback, None);
        assert_eq!(self_drawn.unavailable, None);

        // 关闭：材质种类为「无」，来源轴折叠，什么都不请求。
        let off = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::None, BackdropSource::Auto),
            Some(26100),
            true,
        ));
        assert_eq!(off.material_kind, MaterialKind::None);
        assert_eq!(off.clear_alpha, 1.0);
        assert_eq!(off.material, DwmMaterial::None);
        assert!(!off.root_background_transparent);
        assert!(off.texture.is_none());
    }

    #[test]
    fn every_combination_produces_a_complete_plan() {
        let mut checked = 0usize;
        for material in MaterialKind::ALL {
            for source in BackdropSource::ALL {
                for (build, premultiplied) in CAPABILITIES {
                    let selection = BackdropSelection::new(material, source);
                    let mut input = inputs(selection, build, premultiplied);
                    let plan = BackdropPlan::derive(input);
                    let combination = resolve_combination(
                        selection,
                        &BackdropCapabilities {
                            windows_build: build,
                            premultiplied_alpha_supported: premultiplied,
                            accent_policy_export: true,
                        },
                    );
                    assert_eq!(
                        plan.material_kind, combination.material,
                        "material {material:?} source {source:?} must keep the effective material"
                    );
                    assert_eq!(
                        plan.source, combination.source,
                        "material {material:?} source {source:?} must keep the effective source"
                    );
                    assert_eq!(
                        plan.fallback, combination.fallback,
                        "material {material:?} source {source:?} must keep the fallback reason"
                    );
                    assert_eq!(
                        plan.unavailable, combination.unavailable,
                        "material {material:?} source {source:?} must keep the unavailable reason"
                    );
                    let native = combination.material != MaterialKind::None
                        && combination.source == BackdropSource::Native;
                    let self_drawn = combination.material != MaterialKind::None
                        && combination.source == BackdropSource::SelfDrawn;
                    // 只有原生来源且该材质档确有系统请求时，窗口才允许清成全透明。
                    // 系统请求既可能来自 DWM 通道，也可能来自 accent policy 通道。
                    let request = if native {
                        expected_system_request(combination.material)
                    } else {
                        None
                    };
                    let accent_tint = if native && combination.material == MaterialKind::AcrylicTint
                    {
                        Some(DEFAULT_ACRYLIC_TINT_ARGB)
                    } else {
                        None
                    };
                    let has_system_request = request.is_some() || accent_tint.is_some();
                    assert_eq!(
                        plan.clear_alpha,
                        if has_system_request { 0.0 } else { 1.0 },
                        "material {material:?} source {source:?} must set the clear alpha"
                    );
                    assert_eq!(
                        plan.material,
                        request.unwrap_or(DwmMaterial::None),
                        "material {material:?} source {source:?} must request the mapped system material"
                    );
                    assert_eq!(
                        plan.accent_tint, accent_tint,
                        "material {material:?} source {source:?} must only tint the accent channel"
                    );
                    assert!(
                        !(plan.material != DwmMaterial::None && plan.accent_tint.is_some()),
                        "material {material:?} source {source:?} must never use both channels at once"
                    );
                    assert_eq!(
                        plan.texture.is_some(),
                        self_drawn,
                        "material {material:?} source {source:?} must only produce a texture when self-drawn"
                    );
                    assert_eq!(
                        plan.root_background_transparent,
                        has_system_request,
                        "material {material:?} source {source:?} must stay opaque until the texture is ready"
                    );
                    input.self_drawn_ready = true;
                    let ready = BackdropPlan::derive(input);
                    assert_eq!(
                        ready.root_background_transparent,
                        has_system_request || self_drawn,
                        "material {material:?} source {source:?} must follow the texture readiness"
                    );
                    assert_eq!(
                        plan.composition,
                        if native_baseline(build, premultiplied) {
                            BackdropCompositeAlpha::PreMultiplied
                        } else {
                            BackdropCompositeAlpha::Opaque
                        },
                        "material {material:?} source {source:?} must keep the platform baseline composition"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(
            checked, 210,
            "the plan must be pinned for every material, source and capability case"
        );
    }

    #[test]
    fn the_material_request_table_covers_every_material_kind() {
        assert_eq!(
            MATERIAL_REQUEST_TABLE.len(),
            MaterialKind::ALL.len(),
            "the request table must have one row per material kind"
        );
        for material in MaterialKind::ALL {
            let row = MATERIAL_REQUEST_TABLE
                .iter()
                .find(|row| row.kind == material)
                .expect("every material kind must have a request row");
            assert_eq!(
                row.channel,
                match expected_system_request(material) {
                    Some(request) => MaterialChannel::Dwm(request),
                    None if material == MaterialKind::AcrylicTint => {
                        MaterialChannel::AccentPolicy
                    }
                    None => MaterialChannel::None,
                },
                "{material:?} must map to its own system channel row"
            );
            assert_eq!(
                system_material_request(material),
                expected_system_request(material),
                "{material:?} must be resolved through the request table"
            );
        }
    }

    #[test]
    fn an_unconfirmed_system_channel_keeps_the_root_opaque() {
        // 沉浸式后景的系统取值未被探测确认：原生来源也拿不到系统材质请求，
        // 实际组合退化成「没有后景」，窗口必须保持不透明，并报出不可用原因。
        let plan = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Immersive, BackdropSource::Native),
            Some(26100),
            true,
        ));
        assert_eq!(plan.material_kind, MaterialKind::None);
        assert_eq!(plan.source, BackdropSource::Native);
        assert_eq!(plan.material, DwmMaterial::None);
        assert_eq!(plan.clear_alpha, 1.0);
        assert!(!plan.root_background_transparent);
        assert_eq!(
            plan.unavailable,
            Some(UnavailableReason::SystemChannelUnconfirmed),
            "the plan must surface the unconfirmed system channel"
        );
        assert_eq!(
            plan.composition,
            BackdropCompositeAlpha::PreMultiplied,
            "the surface composition must still follow the platform baseline"
        );
    }

    #[test]
    fn the_accent_tint_tier_takes_its_own_channel() {
        let plan = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::AcrylicTint, BackdropSource::Native),
            Some(26100),
            true,
        ));
        assert_eq!(plan.material_kind, MaterialKind::AcrylicTint);
        assert_eq!(plan.source, BackdropSource::Native);
        // 关键断言：它不占用 DWM 材质取值，而是填 accent 色调。
        assert_eq!(plan.material, DwmMaterial::None);
        assert_eq!(plan.accent_tint, Some(DEFAULT_ACRYLIC_TINT_ARGB));
        assert_eq!(
            plan.clear_alpha, 0.0,
            "the system compositor draws the material, so the clear must be fully transparent"
        );
        assert!(plan.root_background_transparent);
        assert!(plan.texture.is_none());

        // 其它材质照旧走 DWM 通道，且绝不带 accent 色调。
        let mica = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Native),
            Some(26100),
            true,
        ));
        assert_eq!(mica.material, DwmMaterial::Mica);
        assert_eq!(
            mica.accent_tint, None,
            "the accent channel must never leak into the system material channel"
        );

        // 导出解析失败时该档降级，计划里不会出现 accent 色调。
        let mut missing = inputs(
            BackdropSelection::new(MaterialKind::AcrylicTint, BackdropSource::Native),
            Some(26100),
            true,
        );
        missing.accent_policy_export = false;
        let degraded = BackdropPlan::derive(missing);
        assert_eq!(degraded.accent_tint, None);
        assert!(degraded.unavailable.is_some());
    }

    #[test]
    fn root_transparency_follows_texture_readiness_on_the_self_drawn_path() {
        let mut input = inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        );
        assert!(!BackdropPlan::derive(input).root_background_transparent);
        input.self_drawn_ready = true;
        assert!(BackdropPlan::derive(input).root_background_transparent);
    }

    #[test]
    fn composition_only_depends_on_the_platform_baseline() {
        for material in MaterialKind::ALL {
            for source in BackdropSource::ALL {
                let supported = BackdropPlan::derive(inputs(
                    BackdropSelection::new(material, source),
                    Some(26100),
                    true,
                ));
                assert_eq!(
                    supported.composition,
                    BackdropCompositeAlpha::PreMultiplied,
                    "{material:?} {source:?} must keep a premultiplied surface when the platform supports it"
                );
                let unsupported = BackdropPlan::derive(inputs(
                    BackdropSelection::new(material, source),
                    Some(19045),
                    true,
                ));
                assert_eq!(
                    unsupported.composition,
                    BackdropCompositeAlpha::Opaque,
                    "{material:?} {source:?} must stay opaque without native capability"
                );
            }
        }
    }

    #[test]
    fn switching_material_or_source_never_changes_the_composition() {
        for (build, premultiplied) in CAPABILITIES {
            let expected = if native_baseline(build, premultiplied) {
                BackdropCompositeAlpha::PreMultiplied
            } else {
                BackdropCompositeAlpha::Opaque
            };
            for material in MaterialKind::ALL {
                for source in BackdropSource::ALL {
                    let plan = BackdropPlan::derive(inputs(
                        BackdropSelection::new(material, source),
                        build,
                        premultiplied,
                    ));
                    assert_eq!(
                        plan.composition, expected,
                        "material {material:?} source {source:?} must not reconfigure the surface"
                    );
                }
            }
        }
    }

    #[test]
    fn composition_survives_a_full_selection_round_trip() {
        let sequence = [
            (MaterialKind::Mica, BackdropSource::Auto),
            (MaterialKind::Mica, BackdropSource::SelfDrawn),
            (MaterialKind::None, BackdropSource::Auto),
            (MaterialKind::Mica, BackdropSource::Native),
            (MaterialKind::Acrylic, BackdropSource::Auto),
        ];
        let expected = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::Auto),
            Some(26100),
            true,
        ))
        .composition;
        for (material, source) in sequence {
            let plan = BackdropPlan::derive(inputs(
                BackdropSelection::new(material, source),
                Some(26100),
                true,
            ));
            assert_eq!(
                plan.composition, expected,
                "switching to {material:?} {source:?} must not reconfigure the surface"
            );
        }
    }

    #[test]
    fn texture_covers_the_primary_screen_and_samples_the_window_rect() {
        let plan = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        ));
        let texture = plan.texture.expect("self-drawn path needs a texture");
        assert_eq!((texture.width, texture.height), (1920, 1080));
        assert_eq!((texture.screen_width, texture.screen_height), (1920, 1080));
        assert_eq!(texture.dpi_scale, 1.5);
        assert_eq!(texture.material, MaterialKind::Mica);
        assert_eq!(plan.uv, UvRect::new(0.25, 0.25, 0.5, 0.5));
    }

    #[test]
    fn the_texture_carries_the_effective_material_not_the_requested_one() {
        // 云母变体自绘：纹理必须带变体材质，配方因此比标准云母更深。
        let variant = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::MicaAlt, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        ));
        assert_eq!(
            variant
                .texture
                .expect("the variant must draw a texture")
                .material,
            MaterialKind::MicaAlt
        );

        // 亚克力没有自绘变体：解析出的实际材质是标准云母，纹理也必须如此。
        let fallback = BackdropPlan::derive(inputs(
            BackdropSelection::new(MaterialKind::Acrylic, BackdropSource::SelfDrawn),
            Some(19045),
            true,
        ));
        assert_eq!(fallback.material_kind, MaterialKind::Mica);
        assert_eq!(
            fallback
                .texture
                .expect("the fallback must draw a texture")
                .material,
            MaterialKind::Mica,
            "a fallback texture must not keep the unavailable material"
        );
    }

    #[test]
    fn deep_and_light_themes_only_change_the_texture_tint() {
        let mut light_input = inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        );
        light_input.dark = false;
        let mut dark_input = light_input;
        dark_input.dark = true;
        let light = BackdropPlan::derive(light_input);
        let dark = BackdropPlan::derive(dark_input);
        assert_eq!(light.source, dark.source);
        assert_eq!(light.clear_alpha, dark.clear_alpha);
        assert_eq!(light.uv, dark.uv);
        assert_ne!(
            light.texture.map(|texture| texture.dark),
            dark.texture.map(|texture| texture.dark)
        );
    }

    #[test]
    fn uv_follows_several_window_positions_and_sizes() {
        let cases = [
            (
                PixelRect::new(0, 0, 960, 540),
                UvRect::new(0.0, 0.0, 0.5, 0.5),
            ),
            (
                PixelRect::new(960, 540, 960, 540),
                UvRect::new(0.5, 0.5, 0.5, 0.5),
            ),
            (
                PixelRect::new(100, 50, 400, 300),
                UvRect::new(
                    100.0 / 1920.0,
                    50.0 / 1080.0,
                    400.0 / 1920.0,
                    300.0 / 1080.0,
                ),
            ),
        ];
        for (window, expected) in cases {
            assert_eq!(sample_uv(Some(window), Some(SCREEN)), expected);
        }
        // 主屏原点非零时必须减去原点，不能把副屏坐标直接归一化。
        let offset_screen = PixelRect::new(100, 50, 1920, 1080);
        assert_eq!(
            sample_uv(
                Some(PixelRect::new(580, 320, 960, 540)),
                Some(offset_screen)
            ),
            UvRect::new(0.25, 0.25, 0.5, 0.5)
        );
    }

    #[test]
    fn missing_geometry_skips_the_texture_but_keeps_the_path() {
        let mut input = inputs(
            BackdropSelection::new(MaterialKind::Mica, BackdropSource::SelfDrawn),
            Some(26100),
            true,
        );
        input.window_screen_rect = None;
        let plan = BackdropPlan::derive(input);
        assert_eq!(plan.source, BackdropSource::SelfDrawn);
        assert!(plan.texture.is_none());
        assert_eq!(plan.uv, UvRect::FULL);
    }

    #[test]
    fn uv_maps_a_window_inside_the_primary_screen() {
        let uv = sample_uv(Some(PixelRect::new(480, 270, 960, 540)), Some(SCREEN));
        assert_eq!(uv, UvRect::new(0.25, 0.25, 0.5, 0.5));
    }

    #[test]
    fn uv_clamps_a_partially_offscreen_window() {
        let uv = sample_uv(Some(PixelRect::new(-100, -50, 300, 200)), Some(SCREEN));
        assert_eq!(uv, UvRect::new(0.0, 0.0, 200.0 / 1920.0, 150.0 / 1080.0));
    }

    #[test]
    fn uv_falls_back_to_the_whole_texture_without_overlap() {
        assert_eq!(
            sample_uv(Some(PixelRect::new(4000, 3000, 800, 600)), Some(SCREEN)),
            UvRect::FULL
        );
        assert_eq!(sample_uv(None, Some(SCREEN)), UvRect::FULL);
        assert_eq!(sample_uv(Some(SCREEN), None), UvRect::FULL);
    }

    #[test]
    fn a_window_larger_than_the_screen_is_clamped_to_the_full_texture() {
        assert_eq!(
            sample_uv(Some(PixelRect::new(-500, -400, 4000, 3000)), Some(SCREEN)),
            UvRect::FULL
        );
    }
}
