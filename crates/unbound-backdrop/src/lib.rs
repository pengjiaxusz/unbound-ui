//! 壁纸派生的 Mica（云母）背板。
//!
//! Win11 的 Mica 由 DWM 合成：窗口背后是「壁纸模糊 + 主题色调」的材质，
//! 应用的半透明图层叠在它上面。但使用 `OPAQUE` 交换链的 GPU 窗口拿不到
//! DWM 的云母层（整窗被应用自己的像素盖住），因此框架必须**自己画出**
//! 被窗口挡住的那块壁纸。
//!
//! 本 crate 复刻 WinUI 的三步合成：
//!
//! 1. 壁纸按 `cover` 规则铺满主屏，截取窗口所在矩形；
//! 2. 降采样后做 O(n) 盒式模糊（三次近似高斯，等效于 WinUI 的 blur 层）；
//! 3. 明度向主题基准色压缩、色偏稀释（等效于 `TintColor` / `LuminosityOpacity`）。
//!
//! 磨砂强度以**屏幕物理像素**为基准：降采样后的模糊半径按「一个纹理像素等于
//! 多少屏幕物理像素」换算，因此换壁纸分辨率、换屏幕分辨率、换显示缩放都不
//! 改变磨砂观感。
//!
//! 结果是一张覆盖整窗的**不透明** PNG，由 `unbound-app` 提交给资产中心并画在
//! 所有图元之下。生成在后台线程完成（壁纸解码是主要成本），主线程只负责上传。
//!
//! 拿不到静态壁纸（幻灯片、动态壁纸）时返回 `None`，调用方回退到纯色底。
//!
//! 纹理统计接缝 [`texture_statistics`] 输入壁纸像素与目标屏幕像素，输出空间
//! 梯度能量、明度带宽与色相保真度，走的是与真实生成完全相同的配方，用来在无
//! 显示环境里守住磨砂锚点的不变量。

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod mica;
pub mod plan;
pub mod selection;
pub mod state;
pub mod stats;
pub mod wallpaper;

#[cfg(test)]
mod test_support;

pub use mica::{generate, MicaImage, MicaRequest, DEFAULT_MAX_WIDTH, DEFAULT_SIGMA};
pub use plan::{
    material_channel, sample_uv, system_material_request, AcrylicTintPreset,
    BackdropCompositeAlpha, BackdropInputs, BackdropPlan, DwmMaterial, MaterialChannel,
    MaterialRequest, PixelRect, UvRect, WallpaperTexture, ACRYLIC_TINT_PRESETS,
    DEFAULT_ACRYLIC_TINT_ARGB, MATERIAL_REQUEST_TABLE,
};
pub use selection::{
    has_self_drawn_variant, is_illegal_combination, native_available, required_build,
    resolve_combination, selectable, self_drawn_coverage, unavailable_reason, BackdropCapabilities,
    BackdropCombination, BackdropSelection, BackdropSource, FallbackReason, MaterialKind,
    SelfDrawnCoverage, UnavailableReason, ACCENT_POLICY_ACRYLIC_MIN_BUILD,
    IMMERSIVE_BACKDROP_MIN_BUILD, NATIVE_MICA_MIN_BUILD,
};
pub use state::{BackdropGenerator, MICA_READY_EVENT_ID};
pub use stats::{texture_statistics, StatisticsRequest, TextureStatistics};
pub use wallpaper::{
    primary_screen_rect, primary_screen_scale, wallpaper_path, wallpaper_stamp, WallpaperStamp,
};
