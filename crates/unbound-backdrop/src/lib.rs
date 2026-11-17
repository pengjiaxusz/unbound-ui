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
//! 2. 降采样后做 O(n) 盒式模糊（三次近似高斯，等价于 WinUI 的 blur 层）；
//! 3. 明度向主题基准色压缩、色偏稀释（等价于 `TintColor` / `LuminosityOpacity`）。
//!
//! 结果是一张覆盖整窗的**不透明** PNG，由 `unbound-app` 提交给资产中心并画在
//! 所有图元之下。生成在后台线程完成（壁纸解码是主要成本），主线程只负责上传。
//!
//! 拿不到静态壁纸（幻灯片、动态壁纸）时返回 `None`，调用方回退到纯色底。

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod mica;
pub mod mode;
pub mod state;
pub mod wallpaper;

pub use mica::{
    generate, generate_blurred_rect, MicaImage, MicaRequest, DEFAULT_MAX_WIDTH, DEFAULT_SIGMA,
};
pub use mode::{native_backdrop_available, BackdropMode, ResolvedBackdrop, NATIVE_MICA_MIN_BUILD};
pub use state::{BackdropGenerator, MICA_READY_EVENT_ID};
pub use wallpaper::wallpaper_path;
