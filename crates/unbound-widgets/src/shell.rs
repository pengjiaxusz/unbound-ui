//! 应用外壳与展厅通用高阶组件：矢量图标、图标按钮、侧边栏导航项、分组标题、品牌徽标、搜索触发框、特性卡片与首屏横幅。
//!
//! 本模块将原本内联在 `apps/showcase` 中的高颜值外壳与展厅视觉图元下沉为标准控件层组件，
//! 满足三大核心契约：
//! 1. **跨状态图元数量恒定**：悬停、按压、聚焦与选中仅修改图元字段（颜色槽位或透明度），绝不增删图元。
//! 2. **纯语义令牌着色**：所有表面、描边、图标与字形均引用 [`Token`]，主题切换零上传开销。
//! 3. **双模复用**：既提供面向 `Canvas` / 无头渲染器的底层 `emit_*` 函数，也实现声明式 [`UiWidget`]。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_gpu::data::{GpuPayload, GpuPrimitiveInstance, PrimType, SubElement};
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::badge::BadgeVariant;
use crate::button::BUTTON_LAYER;
use crate::ergonomics::{AlignX, AlignY, TextCtx, UiWidget, WidgetInputEvent};
use crate::primitive::{curve_stroke, focus_ring, glyphs, instance, outlined, surface};
use crate::switch::CONTROL_LAYER;
use crate::{ControlState, PointerPhase, WidgetOutput};

/// 外壳与导航栏共用的几何矢量图标词汇表。
///
/// 所有图标均由解析几何图元（圆角矩形、SDF 圆环与贝塞尔线段）绘制，
/// 彻底消除对系统特定符号字体的依赖，避免缺字豆腐块或基线偏移。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShellIcon {
    /// 三横线汉堡菜单图标。
    #[default]
    Menu,
    /// 放大镜搜索图标。
    Search,
    /// 太阳（亮色主题）图标。
    Sun,
    /// 月亮（暗色主题）图标。
    Moon,
    /// 3x3 像素对齐网格图标。
    Snap,
    /// 三柱遥测统计图图标。
    Chart,
    /// 2x2 圆角方块网格图标。
    Grid,
    /// 三层堆叠图层图标。
    Layers,
    /// 向下展开箭头。
    ChevronDown,
    /// 向右折叠箭头。
    ChevronRight,
    /// 勾选对号。
    Check,
    /// 关闭叉号。
    Close,
    /// 新建加号。
    Plus,
    /// 启动/播放三角符号。
    Play,
    /// 撤销左弯箭头。
    Undo,
    /// 重做右弯箭头。
    Redo,
    /// 树节点圆点。
    Dot,
    /// 命令行终端（`>_`）图标。
    Terminal,
    /// 代码括号（`</>`）图标。
    Code,
    /// 参数调节滑块图标。
    Sliders,
    /// 闪电/实时会话图标。
    Bolt,
    /// 芯片/硬件诊断图标。
    Cpu,
    /// 盾牌/安全与冲突诊断图标。
    Shield,
    /// 实验室/星芒图标。
    Sparkles,
    /// 视口/浮动窗预览图标。
    Eye,
    /// 文件夹目录图标。
    Folder,
    /// 向上箭头。
    ArrowUp,
    /// 向下箭头。
    ArrowDown,
    /// 向右箭头。
    ArrowRight,
    /// 火箭：启动预设与发射入口。
    Rocket,
    /// 环境变量（`$` 字形）。
    EnvVars,
    /// 链环：附加方案与外部链接。
    Link,
    /// 键盘：快捷键预设。
    Keyboard,
    /// 包裹箱：组件库与产物包。
    Package,
    /// 雷达扫描：进程发现板。
    Radar,
    /// 脉冲波形：通信通道诊断。
    Pulse,
    /// 带引脚芯片：显卡与硬件诊断。
    GpuChip,
    /// 仪表盘：显存与性能统计。
    Dashboard,
    /// 烧杯：开发实验室。
    Beaker,
    /// 窗口：浮动预览与视口。
    Window,
    /// 齿轮：设置页。
    Settings,
}

/// 将 [`ShellIcon`] 居中发射到 `primitives` 与 `payloads` 中。
#[allow(clippy::too_many_lines)]
pub fn emit_shell_icon_primitives(
    kind: ShellIcon,
    rect: Rect,
    color: Token,
    background: Token,
    z: u16,
    primitives: &mut Vec<GpuPrimitiveInstance>,
    payloads: &mut Vec<GpuPayload>,
) {
    let c = rect.center();
    match kind {
        ShellIcon::Dot => {
            primitives.push(surface(
                Rect::new(c.x - 2.5, c.y - 2.5, 5.0, 5.0),
                2.5,
                color,
                z,
            ));
        }
        ShellIcon::Menu => {
            for i in 0..3 {
                primitives.push(surface(
                    Rect::new(c.x - 7.5, c.y - 5.5 + i as f32 * 4.5, 15.0, 1.8),
                    0.9,
                    color,
                    z,
                ));
            }
        }
        ShellIcon::Search => {
            primitives.push(outlined(
                Rect::new(c.x - 7.5, c.y - 7.5, 11.5, 11.5),
                5.75,
                Token::NONE,
                color,
                1.7,
                0.0,
                z,
            ));
            curve_stroke(
                [c.x + 2.2, c.y + 2.2],
                [c.x + 6.8, c.y + 6.8],
                1.8,
                color,
                z + 1,
                primitives,
                payloads,
            );
        }
        ShellIcon::Sun => {
            primitives.push(surface(
                Rect::new(c.x - 4.5, c.y - 4.5, 9.0, 9.0),
                4.5,
                color,
                z,
            ));
            for (dx, dy, w, h) in [
                (-0.9, -8.5, 1.8, 2.8),
                (-0.9, 5.7, 1.8, 2.8),
                (-8.5, -0.9, 2.8, 1.8),
                (5.7, -0.9, 2.8, 1.8),
            ] {
                primitives.push(surface(Rect::new(c.x + dx, c.y + dy, w, h), 0.9, color, z));
            }
        }
        ShellIcon::Moon => {
            primitives.push(surface(
                Rect::new(c.x - 6.5, c.y - 6.5, 13.0, 13.0),
                6.5,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 1.5, c.y - 7.5, 9.5, 11.5),
                5.75,
                background,
                z + 1,
            ));
        }
        ShellIcon::Snap => {
            for row in 0..3 {
                for col in 0..3 {
                    primitives.push(surface(
                        Rect::new(
                            c.x - 6.5 + col as f32 * 5.0,
                            c.y - 6.5 + row as f32 * 5.0,
                            2.8,
                            2.8,
                        ),
                        1.0,
                        color,
                        z,
                    ));
                }
            }
        }
        ShellIcon::Chart => {
            for (i, height) in [5.5f32, 11.5, 8.5].iter().enumerate() {
                primitives.push(surface(
                    Rect::new(
                        c.x - 6.5 + i as f32 * 5.0,
                        c.y + 5.5 - *height,
                        3.4,
                        *height,
                    ),
                    1.2,
                    color,
                    z,
                ));
            }
        }
        ShellIcon::Grid => {
            for row in 0..2 {
                for col in 0..2 {
                    primitives.push(surface(
                        Rect::new(
                            c.x - 6.5 + col as f32 * 7.5,
                            c.y - 6.5 + row as f32 * 7.5,
                            5.6,
                            5.6,
                        ),
                        1.6,
                        color,
                        z,
                    ));
                }
            }
        }
        ShellIcon::Layers => {
            for i in 0..3 {
                primitives.push(surface(
                    Rect::new(
                        c.x - 7.0 + i as f32 * 1.5,
                        c.y - 5.5 + i as f32 * 4.2,
                        11.0,
                        3.2,
                    ),
                    1.5,
                    color,
                    z,
                ));
            }
        }
        ShellIcon::ChevronDown => {
            curve_stroke(
                [c.x - 4.2, c.y - 1.8],
                [c.x, c.y + 2.4],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x, c.y + 2.4],
                [c.x + 4.2, c.y - 1.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::ChevronRight => {
            curve_stroke(
                [c.x - 1.8, c.y - 4.2],
                [c.x + 2.4, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 2.4, c.y],
                [c.x - 1.8, c.y + 4.2],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Check => {
            curve_stroke(
                [c.x - 4.8, c.y + 0.2],
                [c.x - 1.2, c.y + 3.8],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 1.2, c.y + 3.8],
                [c.x + 5.2, c.y - 3.4],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Close => {
            let half = 4.2;
            curve_stroke(
                [c.x - half, c.y - half],
                [c.x + half, c.y + half],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + half, c.y - half],
                [c.x - half, c.y + half],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Plus => {
            primitives.push(surface(
                Rect::new(c.x - 5.5, c.y - 0.9, 11.0, 1.8),
                0.9,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 0.9, c.y - 5.5, 1.8, 11.0),
                0.9,
                color,
                z,
            ));
        }
        ShellIcon::Play => {
            curve_stroke(
                [c.x - 3.5, c.y - 4.8],
                [c.x + 4.5, c.y],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 4.5, c.y],
                [c.x - 3.5, c.y + 4.8],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 3.5, c.y - 4.8],
                [c.x - 3.5, c.y + 4.8],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Undo => {
            curve_stroke(
                [c.x - 4.5, c.y - 1.0],
                [c.x + 3.5, c.y - 1.0],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 4.5, c.y - 1.0],
                [c.x - 1.2, c.y - 4.2],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 4.5, c.y - 1.0],
                [c.x - 1.2, c.y + 2.2],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Redo => {
            curve_stroke(
                [c.x - 3.5, c.y - 1.0],
                [c.x + 4.5, c.y - 1.0],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 4.5, c.y - 1.0],
                [c.x + 1.2, c.y - 4.2],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 4.5, c.y - 1.0],
                [c.x + 1.2, c.y + 2.2],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Terminal => {
            curve_stroke(
                [c.x - 6.0, c.y - 4.0],
                [c.x - 1.5, c.y],
                1.7,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 1.5, c.y],
                [c.x - 6.0, c.y + 4.0],
                1.7,
                color,
                z,
                primitives,
                payloads,
            );
            primitives.push(surface(
                Rect::new(c.x + 0.5, c.y + 3.0, 5.5, 1.8),
                0.9,
                color,
                z,
            ));
        }
        ShellIcon::Code => {
            curve_stroke(
                [c.x - 2.5, c.y - 4.0],
                [c.x - 6.2, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 6.2, c.y],
                [c.x - 2.5, c.y + 4.0],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 2.5, c.y - 4.0],
                [c.x + 6.2, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 6.2, c.y],
                [c.x + 2.5, c.y + 4.0],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Sliders => {
            primitives.push(surface(
                Rect::new(c.x - 6.5, c.y - 4.0, 13.0, 1.6),
                0.8,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 2.5, c.y - 6.2, 3.6, 6.0),
                1.8,
                color,
                z + 1,
            ));
            primitives.push(surface(
                Rect::new(c.x - 6.5, c.y + 2.6, 13.0, 1.6),
                0.8,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x + 1.0, c.y + 0.4, 3.6, 6.0),
                1.8,
                color,
                z + 1,
            ));
        }
        ShellIcon::Bolt => {
            curve_stroke(
                [c.x + 1.2, c.y - 6.2],
                [c.x - 3.8, c.y + 0.4],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 3.8, c.y + 0.4],
                [c.x + 3.8, c.y - 0.4],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 3.8, c.y - 0.4],
                [c.x - 1.2, c.y + 6.2],
                1.8,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Cpu => {
            primitives.push(outlined(
                Rect::new(c.x - 5.5, c.y - 5.5, 11.0, 11.0),
                2.5,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 2.2, c.y - 2.2, 4.4, 4.4),
                1.0,
                color,
                z + 1,
            ));
        }
        ShellIcon::Shield => {
            primitives.push(outlined(
                Rect::new(c.x - 5.5, c.y - 6.0, 11.0, 12.0),
                4.0,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 0.8, c.y - 3.2, 1.6, 6.4),
                0.8,
                color,
                z + 1,
            ));
        }
        ShellIcon::Sparkles => {
            primitives.push(outlined(
                Rect::new(c.x - 5.0, c.y - 5.0, 10.0, 10.0),
                5.0,
                Token::NONE,
                color,
                1.5,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 2.0, c.y - 2.0, 4.0, 4.0),
                2.0,
                color,
                z + 1,
            ));
        }
        ShellIcon::Eye => {
            primitives.push(outlined(
                Rect::new(c.x - 6.5, c.y - 4.2, 13.0, 8.4),
                4.2,
                Token::NONE,
                color,
                1.5,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 2.0, c.y - 2.0, 4.0, 4.0),
                2.0,
                color,
                z + 1,
            ));
        }
        ShellIcon::Folder => {
            primitives.push(surface(
                Rect::new(c.x - 6.5, c.y - 5.5, 5.5, 2.5),
                1.2,
                color,
                z,
            ));
            primitives.push(outlined(
                Rect::new(c.x - 6.5, c.y - 3.5, 13.0, 9.0),
                2.2,
                Token::NONE,
                color,
                1.6,
                0.0,
                z + 1,
            ));
        }
        ShellIcon::ArrowUp => {
            curve_stroke(
                [c.x, c.y - 4.8],
                [c.x, c.y + 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 3.6, c.y - 1.2],
                [c.x, c.y - 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 3.6, c.y - 1.2],
                [c.x, c.y - 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::ArrowDown => {
            curve_stroke(
                [c.x, c.y - 4.8],
                [c.x, c.y + 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x - 3.6, c.y + 1.2],
                [c.x, c.y + 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 3.6, c.y + 1.2],
                [c.x, c.y + 4.8],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::ArrowRight => {
            curve_stroke(
                [c.x - 5.0, c.y],
                [c.x + 4.5, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 1.0, c.y - 3.6],
                [c.x + 4.5, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 1.0, c.y + 3.6],
                [c.x + 4.5, c.y],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
        }
        ShellIcon::Rocket => {
            // 机身 + 两侧尾翼 + 尾焰。
            primitives.push(surface(
                Rect::new(c.x - 2.2, c.y - 6.4, 4.4, 11.2),
                2.2,
                color,
                z,
            ));
            curve_stroke(
                [c.x - 3.0, c.y + 1.4],
                [c.x - 5.6, c.y + 5.4],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            curve_stroke(
                [c.x + 3.0, c.y + 1.4],
                [c.x + 5.6, c.y + 5.4],
                1.6,
                color,
                z,
                primitives,
                payloads,
            );
            primitives.push(surface(
                Rect::new(c.x - 1.2, c.y + 5.6, 2.4, 2.4),
                1.2,
                color,
                z + 1,
            ));
        }
        ShellIcon::EnvVars => {
            // `$` 字形：竖干 + 上下两条短横。
            primitives.push(surface(
                Rect::new(c.x - 0.9, c.y - 6.6, 1.8, 13.2),
                0.9,
                color,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 4.8, c.y - 4.6, 5.4, 1.8),
                0.9,
                color,
                z + 1,
            ));
            primitives.push(surface(
                Rect::new(c.x - 0.6, c.y - 0.4, 5.4, 1.8),
                0.9,
                color,
                z + 1,
            ));
        }
        ShellIcon::Link => {
            // 两个交叠圆环构成链环。
            primitives.push(outlined(
                Rect::new(c.x - 6.6, c.y - 3.6, 7.6, 7.2),
                3.6,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(outlined(
                Rect::new(c.x - 1.0, c.y - 3.6, 7.6, 7.2),
                3.6,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
        }
        ShellIcon::Keyboard => {
            // 键帽外框 + 三个按键 + 空格条。
            primitives.push(outlined(
                Rect::new(c.x - 7.0, c.y - 4.4, 14.0, 8.8),
                2.0,
                Token::NONE,
                color,
                1.5,
                0.0,
                z,
            ));
            for i in 0..3 {
                primitives.push(surface(
                    Rect::new(c.x - 4.4 + i as f32 * 3.4, c.y - 2.0, 1.8, 1.8),
                    0.5,
                    color,
                    z + 1,
                ));
            }
            primitives.push(surface(
                Rect::new(c.x - 3.6, c.y + 1.2, 7.2, 1.6),
                0.8,
                color,
                z + 1,
            ));
        }
        ShellIcon::Package => {
            // 箱体 + 顶盖 + 中缝胶带。
            primitives.push(outlined(
                Rect::new(c.x - 6.2, c.y - 5.0, 12.4, 11.6),
                2.0,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 6.2, c.y - 5.0, 12.4, 1.8),
                0.9,
                color,
                z + 1,
            ));
            primitives.push(surface(
                Rect::new(c.x - 0.9, c.y - 5.0, 1.8, 4.2),
                0.4,
                color,
                z + 1,
            ));
        }
        ShellIcon::Radar => {
            // 同心扫描环 + 中心目标点。
            primitives.push(outlined(
                Rect::new(c.x - 7.0, c.y - 7.0, 14.0, 14.0),
                7.0,
                Token::NONE,
                color,
                1.5,
                0.0,
                z,
            ));
            primitives.push(outlined(
                Rect::new(c.x - 3.6, c.y - 3.6, 7.2, 7.2),
                3.6,
                Token::NONE,
                color,
                1.4,
                0.0,
                z + 1,
            ));
            primitives.push(surface(
                Rect::new(c.x - 1.2, c.y - 1.2, 2.4, 2.4),
                1.2,
                color,
                z + 2,
            ));
        }
        ShellIcon::Pulse => {
            // 心电波形：基线 → 峰 → 谷 → 基线。
            for (start, end) in [
                ([c.x - 6.6, c.y], [c.x - 3.2, c.y]),
                ([c.x - 3.2, c.y], [c.x - 1.2, c.y - 4.6]),
                ([c.x - 1.2, c.y - 4.6], [c.x + 1.0, c.y + 4.6]),
                ([c.x + 1.0, c.y + 4.6], [c.x + 3.0, c.y]),
                ([c.x + 3.0, c.y], [c.x + 6.6, c.y]),
            ] {
                curve_stroke(start, end, 1.6, color, z, primitives, payloads);
            }
        }
        ShellIcon::GpuChip => {
            // 芯片本体 + 中心核 + 四边引脚。
            primitives.push(outlined(
                Rect::new(c.x - 5.0, c.y - 5.0, 10.0, 10.0),
                2.2,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 2.0, c.y - 2.0, 4.0, 4.0),
                1.0,
                color,
                z + 1,
            ));
            for (dx, dy, w, h) in [
                (-1.4, -7.6, 2.8, 2.0),
                (-1.4, 5.6, 2.8, 2.0),
                (-7.6, -1.4, 2.0, 2.8),
                (5.6, -1.4, 2.0, 2.8),
            ] {
                primitives.push(surface(Rect::new(c.x + dx, c.y + dy, w, h), 0.6, color, z));
            }
        }
        ShellIcon::Dashboard => {
            // 表盘外环 + 指针 + 轴心。
            primitives.push(outlined(
                Rect::new(c.x - 7.0, c.y - 7.0, 14.0, 14.0),
                7.0,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            curve_stroke(
                [c.x, c.y + 1.6],
                [c.x + 4.2, c.y - 3.4],
                1.8,
                color,
                z + 2,
                primitives,
                payloads,
            );
            primitives.push(surface(
                Rect::new(c.x - 1.4, c.y + 0.2, 2.8, 2.8),
                1.4,
                color,
                z + 2,
            ));
        }
        ShellIcon::Beaker => {
            // 瓶颈 + 锥形杯体 + 液面。
            primitives.push(surface(
                Rect::new(c.x - 2.6, c.y - 6.6, 1.8, 4.2),
                0.9,
                color,
                z,
            ));
            primitives.push(outlined(
                Rect::new(c.x - 5.2, c.y - 3.0, 10.4, 9.4),
                2.4,
                Token::NONE,
                color,
                1.6,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 3.6, c.y + 1.6, 7.2, 2.0),
                0.8,
                color,
                z + 1,
            ));
        }
        ShellIcon::Window => {
            // 窗口外框 + 标题栏横带 + 两个窗口按钮。
            primitives.push(outlined(
                Rect::new(c.x - 7.0, c.y - 5.6, 14.0, 11.2),
                2.0,
                Token::NONE,
                color,
                1.5,
                0.0,
                z,
            ));
            primitives.push(surface(
                Rect::new(c.x - 7.0, c.y - 5.6, 14.0, 2.6),
                1.3,
                color,
                z,
            ));
            for (dx, dy) in [(-4.4, -4.6), (2.4, -4.6)] {
                primitives.push(surface(
                    Rect::new(c.x + dx, c.y + dy, 2.0, 0.9),
                    0.45,
                    color,
                    z + 1,
                ));
            }
        }
        ShellIcon::Settings => {
            // 中心圆环 + 四个齿。
            primitives.push(outlined(
                Rect::new(c.x - 4.2, c.y - 4.2, 8.4, 8.4),
                4.2,
                Token::NONE,
                color,
                1.8,
                0.0,
                z,
            ));
            for (dx, dy, w, h) in [
                (-1.2, -7.4, 2.4, 3.0),
                (-1.2, 4.4, 2.4, 3.0),
                (-7.4, -1.2, 3.0, 2.4),
                (4.4, -1.2, 3.0, 2.4),
            ] {
                primitives.push(surface(Rect::new(c.x + dx, c.y + dy, w, h), 0.8, color, z));
            }
        }
    }
}

/// 纯装饰性的 [`ShellIcon`] 控件描述符。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ShellIconDesc {
    /// 图标种类。
    pub icon: ShellIcon,
    /// 尺寸（正方形边长）。
    pub size: f32,
    /// 前景颜色令牌。
    pub color: Token,
    /// 遮罩背景色令牌（仅 `ShellIcon::Moon` 使用）。
    pub background: Token,
    /// 绘制深度。
    pub z: u16,
}

impl ShellIconDesc {
    /// 创建默认 20px 的矢量图标描述符。
    #[must_use]
    pub const fn new(icon: ShellIcon) -> Self {
        Self {
            icon,
            size: 20.0,
            color: Token::TEXT_PRIMARY,
            background: Token::SURFACE_RAISED,
            z: 690,
        }
    }

    /// 设置图标正方形尺寸。
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// 设置图标颜色令牌。
    #[must_use]
    pub const fn color(mut self, color: Token) -> Self {
        self.color = color;
        self
    }

    /// 设置图标背景色令牌。
    #[must_use]
    pub const fn background(mut self, background: Token) -> Self {
        self.background = background;
        self
    }
}

impl UiWidget for ShellIconDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.size.max(12.0), self.size.max(12.0))
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut primitives = Vec::with_capacity(10);
        let mut payloads = Vec::with_capacity(4);
        emit_shell_icon_primitives(
            self.icon,
            rect,
            self.color,
            self.background,
            self.z,
            &mut primitives,
            &mut payloads,
        );
        WidgetOutput {
            primitives,
            payloads,
            ..Default::default()
        }
    }
}

/// 可点击的矢量图标按钮描述符（与 Showcase `widgets::icon_button` 像素与跨度契约完全一致）。
#[derive(Clone, Debug, PartialEq)]
pub struct IconButtonDesc {
    /// 边界矩形（声明式模式下由布局引擎填充）。
    pub rect: Rect,
    /// 图标类型。
    pub icon: ShellIcon,
    /// 无障碍标签。
    pub label: String,
    /// 是否处于激活/选中状态。
    pub selected: bool,
    /// 按钮宽高（声明式固有尺寸）。
    pub size: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 遮罩背景色令牌。
    pub background: Token,
    /// 绘制深度。
    pub z: u16,
}

impl IconButtonDesc {
    /// 创建带图标与无障碍标签的图标按钮。
    #[must_use]
    pub fn new(icon: ShellIcon, label: impl Into<String>) -> Self {
        Self {
            rect: Rect::ZERO,
            icon,
            label: label.into(),
            selected: false,
            size: 32.0,
            radius: 4.0,
            background: Token::CARD_BG,
            z: 702,
        }
    }

    /// 设置显式矩形。
    #[must_use]
    pub const fn with_rect(mut self, rect: Rect) -> Self {
        self.rect = rect;
        self
    }

    /// 设置激活选中状态。
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 设置固有方形边长。
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// 设置圆角半径。
    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

/// 发射图标按钮（底色胶囊 + 矢量图标 + 焦点环，图元数量在所有状态间恒定）。
#[must_use]
pub fn emit_icon_button(node: u64, desc: &IconButtonDesc, control: ControlState) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(12);
    let mut payloads = Vec::with_capacity(4);
    let active_selected = desc.selected || control.selected;
    let fill = match control.phase {
        PointerPhase::Hovered => Token::CONTROL_HOVER,
        PointerPhase::Pressed => Token::NAV_SELECTED,
        PointerPhase::Idle => {
            if active_selected {
                Token::NAV_SELECTED
            } else {
                Token::NONE
            }
        }
    };
    primitives.push(surface(desc.rect, desc.radius, fill, desc.z));
    let fg = if active_selected {
        Token::PRIMARY
    } else {
        Token::TEXT_PRIMARY
    };
    emit_shell_icon_primitives(
        desc.icon,
        desc.rect,
        fg,
        desc.background,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        desc.rect.inflate(1.0),
        desc.radius + 1.0,
        Token::FOCUS_RING,
        2.0,
        desc.z + 2,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Button, desc.label.clone(), desc.rect).clickable();
    a11y.selected = active_selected;
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| {
            HitRegion::new(node, BUTTON_LAYER, u32::from(desc.z), desc.rect)
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

impl UiWidget for IconButtonDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let s = self.size.max(self.rect.width()).max(26.0);
        Vec2::new(s, s)
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        // 吞掉按压事件以防止在标题栏内使用时误触发窗口拖拽。
        matches!(event, WidgetInputEvent::PointerDown { .. })
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        emit_icon_button(node, &desc, control)
    }
}

/// 顶部栏品牌徽标控件（Showcase 左上角带微光阴影的圆角品牌色方块与中心字母）。
#[derive(Clone, Debug, PartialEq)]
pub struct AppBrandBadgeDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 中心单字母或简短符号（如 `"U"` 或 `"C"`）。
    pub symbol: String,
    /// 边长尺寸。
    pub size: f32,
    /// 圆角半径。
    pub radius: f32,
    /// 绘制深度。
    pub z: u16,
}

impl AppBrandBadgeDesc {
    /// 创建带 `symbol` 的品牌徽标。
    #[must_use]
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            rect: Rect::ZERO,
            symbol: symbol.into(),
            size: 26.0,
            radius: 7.0,
            z: 702,
        }
    }

    /// 设置显式尺寸。
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// 设置圆角半径。
    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

/// 发射顶部栏品牌徽标。
#[must_use]
pub fn emit_brand_badge(desc: &AppBrandBadgeDesc, symbol_glyphs: &[GlyphQuad]) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + symbol_glyphs.len());
    let mut payloads = Vec::with_capacity(symbol_glyphs.len());
    primitives.push(outlined(
        desc.rect,
        desc.radius,
        Token::PRIMARY,
        Token::NONE,
        0.0,
        4.0,
        desc.z,
    ));
    glyphs(
        symbol_glyphs,
        Token::ON_PRIMARY,
        desc.z + 1,
        &mut primitives,
        &mut payloads,
    );
    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

impl UiWidget for AppBrandBadgeDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let s = self.size.max(20.0);
        Vec2::new(s, s)
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let quads = text_ctx.shape_line(
            &desc.symbol,
            rect,
            (rect.height() * 0.54).clamp(11.0, 16.0),
            700,
            AlignX::Center,
            AlignY::Middle,
        );
        emit_brand_badge(&desc, &quads)
    }
}

/// 侧边栏分组标题描述符（如 Showcase 的 `DESIGN GUIDANCE`、`BASIC INPUT`）。
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarSectionHeaderDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 分组标题文本。
    pub title: String,
    /// 当前活跃路由是否归属该分组（为 `true` 时使用 `Token::PRIMARY` 高亮）。
    pub active: bool,
    /// 行高（由布局层写入，声明式挂载时作为固有高度）。
    pub height: f32,
    /// 绘制深度。
    pub z: u16,
}

impl SidebarSectionHeaderDesc {
    /// 创建侧边栏分组标题。
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            rect: Rect::ZERO,
            title: title.into(),
            active: false,
            height: 24.0,
            z: 620,
        }
    }

    /// 设置当前分组是否包含激活页面。
    #[must_use]
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// 设置分组标题行高。
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

impl UiWidget for SidebarSectionHeaderDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let m = text_ctx.measure_line(&self.title, 11.5, 600);
        Vec2::new((m.x + 20.0).max(self.rect.width()), 30.0)
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        // 分组标题在行内偏下：上方留白把「新分组」与上一项拉开，下方贴近所属条目。
        let text_rect = Rect::new(
            rect.origin.x + 12.0,
            rect.origin.y + 8.0,
            (rect.width() - 16.0).max(0.0),
            (rect.height() - 8.0).max(16.0),
        );
        let quads = text_ctx.shape_line(
            &self.title,
            text_rect,
            11.5,
            600,
            AlignX::Left,
            AlignY::Middle,
        );
        let mut primitives = Vec::with_capacity(quads.len());
        let mut payloads = Vec::with_capacity(quads.len());
        // Win11 的导航分组标题是中性灰的小字，不用强调色——强调色只属于选中项本身。
        // `active` 仍保留给无障碍与外部状态使用，但不参与着色。
        let color = Token::NAV_TEXT_MUTED;
        glyphs(&quads, color, self.z, &mut primitives, &mut payloads);
        WidgetOutput {
            primitives,
            payloads,
            a11y: Some(A11yNode::new(Role::Heading, self.title.clone(), rect)),
            ..Default::default()
        }
    }
}

/// 侧边栏单行导航项描述符（下沉自 Showcase 的 `nav_item` 与 `draw_collapsed_nav`）。
///
/// 视觉构成：
/// - 8px 圆角胶囊背景（选中为 `Token::NAV_SELECTED`，悬停为 `Token::NAV_HOVER`，空闲为 `Token::NAV_BG`）；
/// - 胶囊内部左侧 `3x16` 圆角主色指示条（选中时 `opacity = 1.0`，未选中时 `opacity = 0.0`，保持图元数恒定）；
/// - 左侧几何矢量 [`ShellIcon`]（选中为 `Token::PRIMARY`，未选中为 `Token::NAV_TEXT_MUTED`）；
/// - 导航文本标签（选中为 `Token::NAV_TEXT` 600 字重，未选中为 `Token::NAV_TEXT_MUTED` 400 字重）；
/// - 可选右侧胶囊徽标（如活跃数量或快捷键提示）。
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarItemDesc {
    /// 边界矩形。
    pub rect: Rect,
    /// 导航项文本标签。
    pub label: String,
    /// 左侧矢量图标。
    pub icon: ShellIcon,
    /// 是否处于当前激活路由。
    pub selected: bool,
    /// 侧边栏是否处于紧凑折叠图标模式。
    pub collapsed: bool,
    /// 子级缩进偏移量（逻辑像素）。
    pub indent: f32,
    /// 可选右侧小型胶囊文本。
    pub badge: Option<String>,
    /// 行高。
    pub height: f32,
    /// 胶囊圆角半径。
    pub radius: f32,
    /// 绘制深度。
    pub z: u16,
}

impl SidebarItemDesc {
    /// 创建侧边栏导航项。
    #[must_use]
    pub fn new(label: impl Into<String>, icon: ShellIcon) -> Self {
        Self {
            rect: Rect::ZERO,
            label: label.into(),
            icon,
            selected: false,
            collapsed: false,
            indent: 0.0,
            badge: None,
            height: 40.0,
            radius: 4.0,
            z: 630,
        }
    }

    /// 设置显式矩形。
    #[must_use]
    pub const fn with_rect(mut self, rect: Rect) -> Self {
        self.rect = rect;
        self
    }

    /// 设置是否选中。
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 设置是否为折叠仅图标模式。
    #[must_use]
    pub const fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// 设置左侧层级缩进。
    #[must_use]
    pub const fn indent(mut self, indent: f32) -> Self {
        self.indent = indent;
        self
    }

    /// 附加右侧徽标文本。
    #[must_use]
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// 设置导航行高度。
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// 设置胶囊圆角半径。
    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 左侧选中强调指示条矩形（`3 x 16`，垂直居中）。
    #[must_use]
    pub fn indicator_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 4.0 + self.indent,
            self.rect.origin.y + (self.rect.size.y - 16.0) * 0.5,
            3.0,
            16.0,
        )
    }

    /// 矢量图标占位矩形（折叠模式下铺满整行以保持水平居中）。
    #[must_use]
    pub fn icon_rect(&self) -> Rect {
        if self.collapsed {
            self.rect
        } else {
            Rect::new(
                self.rect.origin.x + 12.0 + self.indent,
                self.rect.origin.y,
                20.0,
                self.rect.size.y,
            )
        }
    }

    /// 文本标签矩形；`extra_indent` 供调用方在图标与文本之间追加留白。
    #[must_use]
    pub fn label_rect(&self, extra_indent: f32) -> Rect {
        let left = self.rect.origin.x + 38.0 + self.indent + extra_indent;
        Rect::new(
            left,
            self.rect.origin.y,
            (self.rect.max().x - left).max(0.0),
            self.rect.size.y,
        )
    }
}

/// 发射侧边栏导航项（保持跨状态恒定图元数量，支持局部切片修补）。
#[must_use]
pub fn emit_sidebar_item(
    node: u64,
    desc: &SidebarItemDesc,
    control: ControlState,
    label_glyphs: &[GlyphQuad],
    badge_glyphs: &[GlyphQuad],
    badge_rect: Option<Rect>,
) -> WidgetOutput {
    let is_selected = desc.selected || control.selected;
    let is_hovered = matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed);
    let background = if is_selected {
        Token::NAV_SELECTED
    } else if is_hovered {
        Token::NAV_HOVER
    } else {
        Token::NAV_BG
    };

    let mut primitives = Vec::with_capacity(12 + label_glyphs.len() + badge_glyphs.len());
    let mut payloads = Vec::with_capacity(4 + label_glyphs.len() + badge_glyphs.len());

    // 1. 胶囊底色
    primitives.push(surface(desc.rect, desc.radius, background, desc.z));

    // 2. 左侧选中强调指示条（未选中时透明度为 0，保持图元数恒定）
    let mut indicator = surface(desc.indicator_rect(), 1.5, Token::PRIMARY, desc.z + 1);
    indicator.z_and_opacity =
        GpuPrimitiveInstance::pack_z_opacity(desc.z + 1, if is_selected { 1.0 } else { 0.0 });
    primitives.push(indicator);

    // 3. 矢量图标
    let icon_rect = desc.icon_rect();
    let icon_color = if is_selected {
        Token::PRIMARY
    } else if is_hovered {
        Token::NAV_TEXT
    } else {
        Token::NAV_TEXT_MUTED
    };
    emit_shell_icon_primitives(
        desc.icon,
        icon_rect,
        icon_color,
        background,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 4. 文本字形
    if !desc.collapsed && !label_glyphs.is_empty() {
        let text_color = if is_selected {
            Token::NAV_TEXT
        } else {
            Token::NAV_TEXT_MUTED
        };
        glyphs(
            label_glyphs,
            text_color,
            desc.z + 3,
            &mut primitives,
            &mut payloads,
        );
    }

    // 5. 可选右侧胶囊徽标
    if let Some(b_rect) = badge_rect {
        primitives.push(surface(
            b_rect,
            b_rect.size.y * 0.5,
            if is_selected {
                Token::PRIMARY
            } else {
                Token::PRIMARY_SOFT
            },
            desc.z + 3,
        ));
        glyphs(
            badge_glyphs,
            if is_selected {
                Token::ON_PRIMARY
            } else {
                Token::PRIMARY
            },
            desc.z + 4,
            &mut primitives,
            &mut payloads,
        );
    }

    // 6. 键盘焦点环
    primitives.push(focus_ring(
        desc.rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        desc.z + 5,
        control.focused,
    ));

    let mut a11y = A11yNode::new(Role::Link, desc.label.clone(), desc.rect).clickable();
    a11y.selected = is_selected;
    a11y.disabled = control.disabled;

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

impl UiWidget for SidebarItemDesc {
    type State = ();

    fn intrinsic_size(&self, text_ctx: &mut TextCtx<'_>) -> Vec2 {
        let h = self.height.max(self.rect.height()).max(30.0);
        if self.collapsed {
            return Vec2::new(40.0, h);
        }
        let m = text_ctx.measure_line(&self.label, 12.5, if self.selected { 600 } else { 400 });
        let badge_w = self
            .badge
            .as_ref()
            .map_or(0.0, |b| b.len() as f32 * 6.5 + 20.0);
        Vec2::new(
            (46.0 + self.indent + m.x + badge_w)
                .max(self.rect.width())
                .max(120.0),
            h,
        )
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        matches!(event, WidgetInputEvent::PointerDown { .. })
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        if desc.collapsed {
            return emit_sidebar_item(node, &desc, control, &[], &[], None);
        }
        let (badge_rect, badge_glyphs, badge_reserve) = if let Some(badge_str) = &desc.badge {
            let bm = text_ctx.measure_line(badge_str, 10.5, 600);
            let bw = (bm.x + 12.0).max(20.0);
            let bh = 18.0;
            let br = Rect::new(
                rect.max().x - bw - 8.0,
                rect.origin.y + (rect.size.y - bh) * 0.5,
                bw,
                bh,
            );
            let bg = text_ctx.shape_line(badge_str, br, 10.5, 600, AlignX::Center, AlignY::Middle);
            (Some(br), bg, bw + 12.0)
        } else {
            (None, Vec::new(), 6.0)
        };
        let base = desc.label_rect(0.0);
        let text_rect = Rect::new(
            base.origin.x,
            base.origin.y,
            (base.size.x - badge_reserve).max(0.0),
            base.size.y,
        );
        let weight = if desc.selected || control.selected {
            600
        } else {
            400
        };
        let label_glyphs = text_ctx.shape_line(
            &desc.label,
            text_rect,
            12.5,
            weight,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_sidebar_item(
            node,
            &desc,
            control,
            &label_glyphs,
            &badge_glyphs,
            badge_rect,
        )
    }
}

/// 展厅特性/能力卡片描述符（下沉自 Showcase `pages/home.rs::capability_grid`）。
///
/// 包含：
/// - 12px 圆角与 4px 阴影的卡片表面（悬停时高亮为 `Token::CONTROL_HOVER`）；
/// - 左上角 `36x36` 柔和语义色圆角图标底座 + 居中 [`ShellIcon`]；
/// - 标题与副标题；
/// - 右侧可选徽标或 `→` 跳转箭头。
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureCardDesc {
    /// 卡片边界矩形。
    pub rect: Rect,
    /// 标题。
    pub title: String,
    /// 副标题描述。
    pub subtitle: String,
    /// 左侧矢量图标。
    pub icon: ShellIcon,
    /// 图标前景强调色令牌。
    pub accent_fg: Token,
    /// 图标底座底色令牌。
    pub accent_bg: Token,
    /// 是否处于激活选中状态。
    pub selected: bool,
    /// 卡片高度（由布局层写入，声明式挂载时作为固有高度）。
    pub height: f32,
    /// 卡片圆角半径。
    pub radius: f32,
    /// 绘制深度。
    pub z: u16,
}

impl FeatureCardDesc {
    /// 创建特性卡片描述符（默认主强调色图标底座）。
    #[must_use]
    pub fn new(title: impl Into<String>, subtitle: impl Into<String>, icon: ShellIcon) -> Self {
        Self {
            rect: Rect::ZERO,
            title: title.into(),
            subtitle: subtitle.into(),
            icon,
            accent_fg: Token::PRIMARY,
            accent_bg: Token::PRIMARY_SOFT,
            selected: false,
            height: 84.0,
            radius: 12.0,
            z: 610,
        }
    }

    /// 设置显式矩形。
    #[must_use]
    pub const fn with_rect(mut self, rect: Rect) -> Self {
        self.rect = rect;
        self
    }

    /// 设置卡片高度。
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// 设置卡片圆角半径。
    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 显式指定图标前景与底座色令牌。
    #[must_use]
    pub const fn accent(mut self, foreground: Token, background: Token) -> Self {
        self.accent_fg = foreground;
        self.accent_bg = background;
        self
    }

    /// 按语义色系快捷设置图标底座（底座取浅色填充，图标取饱和文本色）。
    #[must_use]
    pub fn tone(mut self, tone: BadgeVariant) -> Self {
        self.accent_fg = tone.text();
        self.accent_bg = tone.fill();
        self
    }

    /// 设置选中高亮态。
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 左上角 `36x36` 图标底座矩形。
    #[must_use]
    pub fn icon_box_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 14.0,
            self.rect.origin.y + 14.0,
            36.0,
            36.0,
        )
    }

    /// 标题文本矩形（右侧留出 28px 箭头净空）。
    #[must_use]
    pub fn title_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 60.0,
            self.rect.origin.y + 15.0,
            (self.rect.size.x - 104.0).max(0.0),
            20.0,
        )
    }

    /// 副标题文本矩形（可排版在卡片剩余高度内，右侧同样留出箭头净空）。
    #[must_use]
    pub fn subtitle_rect(&self) -> Rect {
        Rect::new(
            self.rect.origin.x + 60.0,
            self.rect.origin.y + 38.0,
            (self.rect.size.x - 104.0).max(0.0),
            (self.rect.size.y - 52.0).max(16.0),
        )
    }

    /// 右下角跳转箭头矩形。
    #[must_use]
    pub fn arrow_rect(&self) -> Rect {
        Rect::new(
            self.rect.max().x - 32.0,
            self.rect.max().y - 32.0,
            20.0,
            20.0,
        )
    }
}

/// 发射特性卡片。
#[must_use]
pub fn emit_feature_card(
    node: u64,
    desc: &FeatureCardDesc,
    control: ControlState,
    title_glyphs: &[GlyphQuad],
    subtitle_glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let hovered = matches!(control.phase, PointerPhase::Hovered | PointerPhase::Pressed);
    let selected = desc.selected || control.selected;
    let fill = if hovered {
        Token::CONTROL_HOVER
    } else if selected {
        Token::PRIMARY_SOFT
    } else {
        Token::CARD_BG
    };
    let border = if selected {
        Token::PRIMARY
    } else {
        Token::CARD_BORDER
    };

    let mut primitives = Vec::with_capacity(14 + title_glyphs.len() + subtitle_glyphs.len());
    let mut payloads = Vec::with_capacity(6 + title_glyphs.len() + subtitle_glyphs.len());

    // 卡片主体表面
    primitives.push(outlined(
        desc.rect,
        desc.radius,
        fill,
        border,
        if selected { 1.5 } else { 1.0 },
        4.0,
        desc.z,
    ));

    // 左上角 36x36 圆角语义色图标底座
    let icon_box = desc.icon_box_rect();
    primitives.push(surface(icon_box, 9.0, desc.accent_bg, desc.z + 1));
    emit_shell_icon_primitives(
        desc.icon,
        icon_box,
        desc.accent_fg,
        desc.accent_bg,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 标题与副标题字形
    glyphs(
        title_glyphs,
        Token::TEXT_PRIMARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );
    glyphs(
        subtitle_glyphs,
        Token::TEXT_SECONDARY,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 右下角导航箭头
    let arrow_box = desc.arrow_rect();
    emit_shell_icon_primitives(
        ShellIcon::ArrowRight,
        arrow_box,
        Token::PRIMARY,
        fill,
        desc.z + 2,
        &mut primitives,
        &mut payloads,
    );

    // 焦点环
    primitives.push(focus_ring(
        desc.rect,
        desc.radius,
        Token::FOCUS_RING,
        2.0,
        desc.z + 3,
        control.focused,
    ));

    let mut a11y = A11yNode::new(
        Role::Link,
        format!("{} — {}", desc.title, desc.subtitle),
        desc.rect,
    )
    .clickable();
    a11y.selected = selected;

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled).then(|| {
            HitRegion::new(node, CONTROL_LAYER, u32::from(desc.z), desc.rect)
                .with_cursor(CursorKind::Pointer)
        }),
        a11y: Some(a11y),
        focusable: !control.disabled,
        ..Default::default()
    }
}

impl UiWidget for FeatureCardDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(self.rect.width().max(240.0), self.rect.height().max(84.0))
    }

    fn reduce(&self, _state: &mut Self::State, _rect: Rect, event: &WidgetInputEvent) -> bool {
        matches!(event, WidgetInputEvent::PointerDown { .. })
    }

    fn emit_laid_out(
        &self,
        node: u64,
        rect: Rect,
        control: ControlState,
        _state: &Self::State,
        text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        let mut desc = self.clone();
        desc.rect = rect;
        let title_rect = desc.title_rect();
        let subtitle_rect = desc.subtitle_rect();
        // 单行排版不换行也不裁切，超出矩形宽度的文案会直接压到右下角箭头上；
        // 这里先按可用宽度做省略号截断，保证任何语言的文案都不会溢出版面。
        let title = ellipsize(text_ctx, &desc.title, 14.0, 600, title_rect.width());
        let subtitle = ellipsize(text_ctx, &desc.subtitle, 11.5, 400, subtitle_rect.width());
        let title_glyphs =
            text_ctx.shape_line(&title, title_rect, 14.0, 600, AlignX::Left, AlignY::Middle);
        let subtitle_glyphs = text_ctx.shape_line(
            &subtitle,
            subtitle_rect,
            11.5,
            400,
            AlignX::Left,
            AlignY::Middle,
        );
        emit_feature_card(node, &desc, control, &title_glyphs, &subtitle_glyphs)
    }
}

/// 把单行文案截断到 `max_width` 以内，超出时以省略号收尾。
///
/// [`TextCtx::shape_line`] 只负责按矩形原点排版，不做换行与裁剪；控件需要这层
/// 宽度约束才能保证长文案不会覆盖同一行右侧的装饰图元。
fn ellipsize(
    text_ctx: &mut TextCtx<'_>,
    text: &str,
    size: f32,
    weight: u16,
    max_width: f32,
) -> String {
    if max_width <= 0.0 || text.is_empty() {
        return String::new();
    }
    if text_ctx.measure_line(text, size, weight).x <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high).div_ceil(2);
        let mut candidate: String = chars[..mid].iter().collect();
        candidate.push('…');
        if text_ctx.measure_line(&candidate, size, weight).x <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut out: String = chars[..low].iter().collect();
    out.push('…');
    out
}

/// 首屏渐变光斑与贝塞尔丝带装饰背景控件（下沉自 Showcase `pages/home.rs::hero`）。
///
/// 挂载为绝对定位背景层时，可在上方自由组合声明式标题、胶囊徽标与 `OnHero` 按钮。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HeroBackdropDesc {
    /// 圆角半径。
    pub radius: f32,
    /// 绘制深度。
    pub z: u16,
}

impl Default for HeroBackdropDesc {
    fn default() -> Self {
        Self::new(16.0)
    }
}

impl HeroBackdropDesc {
    /// 创建首屏横幅装饰背景描述符。
    #[must_use]
    pub const fn new(radius: f32) -> Self {
        Self { radius, z: 120 }
    }
}

/// 在 `rect` 内发射 Showcase 首屏渐变光斑与 4 条装饰贝塞尔丝带。
#[must_use]
pub fn emit_hero_backdrop(rect: Rect, desc: &HeroBackdropDesc) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(8);
    let mut payloads = Vec::with_capacity(4);
    let z = desc.z;

    // 1. 品牌渐变主底色表面
    primitives.push(outlined(
        rect,
        desc.radius,
        Token::HERO_START,
        Token::NONE,
        0.0,
        18.0,
        z,
    ));

    // 2. 右侧柔和大光斑（控制在 rect 内部或柔和贴边）
    let blob1_w = (rect.size.x * 0.46).max(80.0);
    let blob1_h = (rect.size.y * 0.88).max(40.0);
    let blob1 = Rect::new(
        rect.max().x - blob1_w - 6.0,
        rect.origin.y + 6.0,
        blob1_w,
        blob1_h,
    );
    primitives.push(surface(blob1, blob1_h * 0.48, Token::HERO_END, z + 1));

    let blob2_w = (rect.size.x * 0.24).max(48.0);
    let blob2_h = (rect.size.y * 0.44).max(24.0);
    let blob2 = Rect::new(
        rect.max().x - blob2_w - 18.0,
        rect.max().y - blob2_h - 6.0,
        blob2_w,
        blob2_h,
    );
    primitives.push(surface(blob2, blob2_h * 0.5, Token::SECONDARY, z + 2));

    // 3. 右下角 4 条解析贝塞尔装饰丝带
    for index in 0..4 {
        let f = index as f32;
        let p0 = [
            rect.origin.x + rect.size.x * (0.58 + f * 0.03),
            rect.max().y - 4.0,
        ];
        let p1 = [
            rect.origin.x + rect.size.x * 0.82,
            rect.origin.y + rect.size.y * 0.42 + f * 10.0,
        ];
        let p2 = [
            rect.max().x - 8.0,
            rect.origin.y + rect.size.y * 0.78 - f * 9.0,
        ];
        let min_x = p0[0].min(p1[0]).min(p2[0]) - 3.0;
        let min_y = p0[1].min(p1[1]).min(p2[1]) - 3.0;
        let max_x = p0[0].max(p1[0]).max(p2[0]) + 3.0;
        let max_y = p0[1].max(p1[1]).max(p2[1]) + 3.0;
        let bounds = Rect::new(
            min_x,
            min_y,
            (max_x - min_x).max(1.0),
            (max_y - min_y).max(1.0),
        );
        let payload_idx = payloads.len() as u32;
        payloads.push(GpuPayload::curve(
            [p0[0] - bounds.origin.x, p0[1] - bounds.origin.y],
            [p1[0] - bounds.origin.x, p1[1] - bounds.origin.y],
            [p2[0] - bounds.origin.x, p2[1] - bounds.origin.y],
            1.75,
        ));
        let mut prim = instance(
            bounds,
            [0.0; 4],
            Token::ON_HERO,
            Token::NONE,
            0.0,
            0.0,
            PrimType::Curve,
            SubElement::Content,
            z + 3,
        );
        prim.extra_payload_idx = payload_idx;
        primitives.push(prim);
    }

    WidgetOutput {
        primitives,
        payloads,
        ..Default::default()
    }
}

impl UiWidget for HeroBackdropDesc {
    type State = ();

    fn intrinsic_size(&self, _text_ctx: &mut TextCtx<'_>) -> Vec2 {
        Vec2::new(1.0, 1.0)
    }

    fn emit_laid_out(
        &self,
        _node: u64,
        rect: Rect,
        _control: ControlState,
        _state: &Self::State,
        _text_ctx: &mut TextCtx<'_>,
    ) -> WidgetOutput {
        emit_hero_backdrop(rect, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_item_keeps_constant_primitive_count_across_states() {
        let desc = SidebarItemDesc::new("Home", ShellIcon::Grid)
            .with_rect(Rect::new(0.0, 0.0, 220.0, 34.0));
        let mut counts = Vec::new();
        for phase in [
            PointerPhase::Idle,
            PointerPhase::Hovered,
            PointerPhase::Pressed,
        ] {
            for focused in [false, true] {
                for selected in [false, true] {
                    let control = ControlState {
                        phase,
                        focused,
                        disabled: false,
                        selected,
                    };
                    let out = emit_sidebar_item(1, &desc, control, &[], &[], None);
                    counts.push(out.primitives.len());
                }
            }
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]), "{counts:?}");
    }

    #[test]
    fn icon_button_keeps_constant_primitive_count_across_states() {
        let desc = IconButtonDesc::new(ShellIcon::Menu, "Toggle navigation")
            .with_rect(Rect::new(0.0, 0.0, 32.0, 32.0));
        let idle = emit_icon_button(1, &desc, ControlState::default());
        let hovered = emit_icon_button(
            1,
            &desc,
            ControlState {
                phase: PointerPhase::Hovered,
                ..ControlState::default()
            },
        );
        let focused = emit_icon_button(
            1,
            &desc,
            ControlState {
                focused: true,
                ..ControlState::default()
            },
        );
        assert_eq!(idle.primitives.len(), hovered.primitives.len());
        assert_eq!(idle.primitives.len(), focused.primitives.len());
    }
}
