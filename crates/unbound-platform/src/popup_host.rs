//! 越界无焦点原生 Popup 窗口宿主（纯逻辑定位、差分注册表与 Win32 样式位）。
//!
//! 让超出主窗口客户区边界的菜单、下拉列表与气泡浮层能够在独立无边框原生弹出窗中完整显示：
//!
//! * [`clamp_popup_to_work_area`] 在多显示器（含负坐标辅屏）与超大弹出窗等边界条件下，
//!   将期望屏幕矩形安全裁剪至目标显示器工作区（任务栏之外）；
//! * [`PopupHostRegistry`] 以确定性顺序维护业务浮层 ID（`u64`）与原生弹出窗 [`WindowId`]、
//!   宿主 `owner` 及 [`PopupWindowConfig`] 的映射，生成增量同步动作（[`PopupSyncAction`]）
//!   并在宿主最小化、隐藏或关闭时级联回收全部归属弹出窗。

use crate::window::ScreenRect;
use crate::windows::WindowId;

/// Win32 原生弹出窗样式位常量（独立于目标平台暴露，便于无头单测验证）。
pub mod popup_style_bits {
    /// 无边框弹出窗主样式（`WS_POPUP`）。
    pub const WS_POPUP: u32 = 0x8000_0000;
    /// 始终置顶（`WS_EX_TOPMOST`）。
    pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
    /// 工具窗口：不出现在任务栏与 Alt+Tab 切换器中（`WS_EX_TOOLWINDOW`）。
    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    /// 显示或点击时不抢占前台激活态与主窗口键盘/IME 焦点（`WS_EX_NOACTIVATE`）。
    pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
}

/// 越界无焦点原生弹出窗配置。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PopupWindowConfig {
    /// 归属的宿主窗口 ID（用于 Win32 owner 关联、Wayland `xdg_popup` 父表面绑定与级联关闭）。
    pub owner: WindowId,
    /// 弹出窗期望的屏幕物理像素矩形（Win32 绝对坐标；Wayland 作为期望尺寸与回退锚点）。
    pub screen_rect: ScreenRect,
    /// 可选的相对父窗口锚点矩形（物理像素；Wayland 直接映射至 `xdg_positioner::set_anchor_rect`）。
    pub anchor_rect: Option<ScreenRect>,
    /// 是否置顶（`WS_EX_TOPMOST`）。
    pub topmost: bool,
    /// 是否按目标显示器工作区自动防越界裁剪。
    pub clamp_to_work_area: bool,
    /// 是否启用鼠标穿透（`ToolTip` 等只读提示为 `true`，Wayland 下设置空输入区且不抢焦点）。
    pub click_through: bool,
}

impl PopupWindowConfig {
    /// 以默认置顶且启用工作区防越界裁剪的配置构造弹出窗规格。
    #[must_use]
    pub const fn new(owner: WindowId, screen_rect: ScreenRect) -> Self {
        Self {
            owner,
            screen_rect,
            anchor_rect: None,
            topmost: true,
            clamp_to_work_area: true,
            click_through: false,
        }
    }

    /// 附加相对父窗口的锚点矩形（供 Wayland `xdg_positioner` 贴边翻转与 Win32 原点换算）。
    #[must_use]
    pub const fn with_anchor_rect(mut self, anchor_rect: ScreenRect) -> Self {
        self.anchor_rect = Some(anchor_rect);
        self
    }

    /// 设置是否鼠标穿透（`ToolTip` 应设为 `true`）。
    #[must_use]
    pub const fn with_click_through(mut self, click_through: bool) -> Self {
        self.click_through = click_through;
        self
    }

    /// 将相对父窗口锚点按父窗屏幕原点换算为全局屏幕矩形（供 Win32 后端复用）。
    #[must_use]
    pub const fn resolve_global_rect(&self, owner_origin: (i32, i32)) -> ScreenRect {
        if let Some(anchor) = self.anchor_rect {
            ScreenRect::new(
                owner_origin.0.saturating_add(anchor.x),
                owner_origin.1.saturating_add(anchor.y),
                self.screen_rect.width,
                self.screen_rect.height,
            )
        } else {
            self.screen_rect
        }
    }

    /// 计算对应的 Win32 基础窗口样式位（恒为 `WS_POPUP`）。
    #[must_use]
    pub const fn win32_style_bits(&self) -> u32 {
        popup_style_bits::WS_POPUP
    }

    /// 计算对应的 Win32 扩展窗口样式位（`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`，按需叠加 `WS_EX_TOPMOST`）。
    #[must_use]
    pub const fn win32_ex_style_bits(&self) -> u32 {
        let mut bits = popup_style_bits::WS_EX_NOACTIVATE | popup_style_bits::WS_EX_TOOLWINDOW;
        if self.topmost {
            bits |= popup_style_bits::WS_EX_TOPMOST;
        }
        bits
    }
}

/// 将期望的弹出窗屏幕矩形裁剪到目标显示器工作区（扣除任务栏）之内。
///
/// 规则：
/// 1. 优先贴近 `desired` 原始锚点，仅在某边越出 `work_area` 时向工作区内平移；
/// 2. 当弹出窗宽或高大于工作区尺寸时，将对应边尺寸限制为工作区尺寸，并将左上角对齐工作区左上角，
///    确保弹出窗左上角始终可见且不会产生坐标溢出或负宽高；
/// 3. 全程使用 64 位有符号算术，安全兼容多显示器负坐标屏幕与极端坐标值。
#[must_use]
pub fn clamp_popup_to_work_area(desired: ScreenRect, work_area: ScreenRect) -> ScreenRect {
    if work_area.is_empty() {
        return desired;
    }

    let wa_left = i64::from(work_area.x);
    let wa_top = i64::from(work_area.y);
    let wa_w = i64::from(work_area.width);
    let wa_h = i64::from(work_area.height);
    let wa_right = wa_left + wa_w;
    let wa_bottom = wa_top + wa_h;

    let (x, width) = if i64::from(desired.width) >= wa_w {
        (wa_left, work_area.width)
    } else {
        let max_x = wa_right - i64::from(desired.width);
        let clamped_x = i64::from(desired.x).clamp(wa_left, max_x);
        (clamped_x, desired.width)
    };

    let (y, height) = if i64::from(desired.height) >= wa_h {
        (wa_top, work_area.height)
    } else {
        let max_y = wa_bottom - i64::from(desired.height);
        let clamped_y = i64::from(desired.y).clamp(wa_top, max_y);
        (clamped_y, desired.height)
    };

    ScreenRect::new(
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width,
        height,
    )
}

/// 已挂载的原生弹出窗注册表条目。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PopupHostEntry {
    /// 应用侧浮层唯一标识。
    pub id: u64,
    /// 平台层为该弹出窗分配的逻辑窗口 ID。
    pub window_id: WindowId,
    /// 归属的宿主窗口 ID。
    pub owner: WindowId,
    /// 当前生效的弹出窗配置。
    pub config: PopupWindowConfig,
}

/// 同步原生弹出窗状态时产出的增量动作。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PopupSyncAction {
    /// 首次创建原生弹出窗。
    Create {
        id: u64,
        window_id: WindowId,
        config: PopupWindowConfig,
    },
    /// 已有弹出窗的位置、尺寸、置顶或归属配置发生变化，需要重定位。
    Reposition {
        id: u64,
        window_id: WindowId,
        config: PopupWindowConfig,
    },
    /// 关闭并销毁原生弹出窗。
    Close { id: u64, window_id: WindowId },
}

/// 纯逻辑原生弹出窗注册表：跟踪浮层 ID 到弹出窗窗口 ID 的映射并生成增量同步计划。
#[derive(Clone, Debug)]
pub struct PopupHostRegistry {
    entries: Vec<PopupHostEntry>,
    next_window_id: u64,
}

impl Default for PopupHostRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PopupHostRegistry {
    /// 创建空注册表（窗口 ID 从 `1` 递增分配）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_window_id: 1,
        }
    }

    /// 指定起始窗口 ID 创建注册表（便于与常规窗口 ID 区间隔离）。
    #[must_use]
    pub const fn with_starting_window_id(start: u64) -> Self {
        Self {
            entries: Vec::new(),
            next_window_id: start,
        }
    }

    /// 当前活跃的弹出窗数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否不存在任何活跃弹出窗。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按业务浮层 ID 查询条目。
    #[must_use]
    pub fn get(&self, id: u64) -> Option<&PopupHostEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// 按平台窗口 ID 查询条目。
    #[must_use]
    pub fn get_by_window(&self, window_id: WindowId) -> Option<&PopupHostEntry> {
        self.entries
            .iter()
            .find(|entry| entry.window_id == window_id)
    }

    /// 将已注册浮层条目绑定到平台实际创建的原生窗口 ID，供运行器按窗口 ID 反查。
    pub fn bind_native_window(&mut self, id: u64, window_id: WindowId) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.window_id = window_id;
            true
        } else {
            false
        }
    }

    /// 按注册顺序遍历全部活跃弹出窗。
    pub fn iter(&self) -> impl Iterator<Item = &PopupHostEntry> {
        self.entries.iter()
    }

    /// 同步单个弹出窗：首次出现时产出 [`PopupSyncAction::Create`]，配置变化时产出
    /// [`PopupSyncAction::Reposition`]，配置未变化时返回 `None`。
    pub fn sync_popup(&mut self, id: u64, config: PopupWindowConfig) -> Option<PopupSyncAction> {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            if entry.config == config {
                return None;
            }
            entry.owner = config.owner;
            entry.config = config;
            return Some(PopupSyncAction::Reposition {
                id,
                window_id: entry.window_id,
                config,
            });
        }

        let window_id = WindowId(self.next_window_id);
        self.next_window_id = self.next_window_id.saturating_add(1);
        self.entries.push(PopupHostEntry {
            id,
            window_id,
            owner: config.owner,
            config,
        });
        Some(PopupSyncAction::Create {
            id,
            window_id,
            config,
        })
    }

    /// 按目标期望集合执行批量差分同步：关闭不再存在的弹出窗，并对新增/变更条目产出创建或重定位动作。
    pub fn sync_desired(&mut self, desired: &[(u64, PopupWindowConfig)]) -> Vec<PopupSyncAction> {
        let mut actions = Vec::new();

        // 先回收不再出现在目标集合中的弹出窗
        let mut keep = Vec::with_capacity(self.entries.len());
        for entry in self.entries.drain(..) {
            if desired.iter().any(|(id, _)| *id == entry.id) {
                keep.push(entry);
            } else {
                actions.push(PopupSyncAction::Close {
                    id: entry.id,
                    window_id: entry.window_id,
                });
            }
        }
        self.entries = keep;

        // 再逐项创建或重定位
        for &(id, config) in desired {
            if let Some(action) = self.sync_popup(id, config) {
                actions.push(action);
            }
        }

        actions
    }

    /// 关闭指定业务浮层 ID 对应的原生弹出窗。
    pub fn close_popup(&mut self, id: u64) -> Option<PopupSyncAction> {
        let pos = self.entries.iter().position(|entry| entry.id == id)?;
        let entry = self.entries.remove(pos);
        Some(PopupSyncAction::Close {
            id: entry.id,
            window_id: entry.window_id,
        })
    }

    /// 级联关闭归属于 `owner` 宿主窗口的全部原生弹出窗（宿主最小化、隐藏或关闭时调用）。
    pub fn close_for_owner(&mut self, owner: WindowId) -> Vec<PopupSyncAction> {
        let mut actions = Vec::new();
        let mut keep = Vec::with_capacity(self.entries.len());
        for entry in self.entries.drain(..) {
            if entry.owner == owner {
                actions.push(PopupSyncAction::Close {
                    id: entry.id,
                    window_id: entry.window_id,
                });
            } else {
                keep.push(entry);
            }
        }
        self.entries = keep;
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_popup_keeps_rect_inside_primary_work_area() {
        let work_area = ScreenRect::new(0, 0, 1920, 1040);

        // 完全在工作区内：原样返回
        let inside = ScreenRect::new(200, 150, 320, 240);
        assert_eq!(clamp_popup_to_work_area(inside, work_area), inside);

        // 右下越界：向内拉回贴边
        let overflow_br = ScreenRect::new(1750, 920, 320, 240);
        assert_eq!(
            clamp_popup_to_work_area(overflow_br, work_area),
            ScreenRect::new(1600, 800, 320, 240)
        );

        // 左上越界：拉回至工作区原点
        let overflow_tl = ScreenRect::new(-40, -25, 320, 240);
        assert_eq!(
            clamp_popup_to_work_area(overflow_tl, work_area),
            ScreenRect::new(0, 0, 320, 240)
        );
    }

    #[test]
    fn clamp_popup_handles_negative_coordinate_secondary_monitor_and_oversized_popup() {
        // 位于主屏左侧的辅屏（X 坐标全为负）
        let left_monitor = ScreenRect::new(-1920, 0, 1920, 1080);

        // 超出辅屏左边界
        let off_left = ScreenRect::new(-2050, -30, 300, 220);
        assert_eq!(
            clamp_popup_to_work_area(off_left, left_monitor),
            ScreenRect::new(-1920, 0, 300, 220)
        );

        // 超出辅屏右边界（x = 0）
        let off_right = ScreenRect::new(-120, 950, 300, 220);
        assert_eq!(
            clamp_popup_to_work_area(off_right, left_monitor),
            ScreenRect::new(-300, 860, 300, 220)
        );

        // 超大弹出窗：限制尺寸并保持左上角对齐工作区左上角
        let huge = ScreenRect::new(-1500, 200, 3000, 2000);
        assert_eq!(
            clamp_popup_to_work_area(huge, left_monitor),
            ScreenRect::new(-1920, 0, 1920, 1080)
        );
    }

    #[test]
    fn popup_config_emits_noactivate_toolwindow_and_optional_topmost_bits() {
        let cfg = PopupWindowConfig::new(WindowId(1), ScreenRect::new(100, 100, 240, 180));
        assert_eq!(cfg.win32_style_bits(), popup_style_bits::WS_POPUP);
        let ex = cfg.win32_ex_style_bits();
        assert_ne!(ex & popup_style_bits::WS_EX_NOACTIVATE, 0);
        assert_ne!(ex & popup_style_bits::WS_EX_TOOLWINDOW, 0);
        assert_ne!(ex & popup_style_bits::WS_EX_TOPMOST, 0);

        let non_topmost = PopupWindowConfig {
            topmost: false,
            ..cfg
        };
        assert_eq!(
            non_topmost.win32_ex_style_bits() & popup_style_bits::WS_EX_TOPMOST,
            0
        );
    }

    #[test]
    fn popup_registry_computes_create_reposition_close_and_owner_cascade() {
        let mut reg = PopupHostRegistry::new();
        let owner_a = WindowId(10);
        let owner_b = WindowId(20);
        let cfg_1 = PopupWindowConfig::new(owner_a, ScreenRect::new(100, 100, 200, 160));
        let cfg_2 = PopupWindowConfig::new(owner_a, ScreenRect::new(300, 200, 220, 140));
        let cfg_b = PopupWindowConfig::new(owner_b, ScreenRect::new(500, 300, 180, 120));

        // 首次同步 -> Create
        let created = reg.sync_popup(1, cfg_1).expect("create");
        let PopupSyncAction::Create {
            id: 1,
            window_id: win_1,
            config,
        } = created
        else {
            panic!("expected Create");
        };
        assert_eq!(config, cfg_1);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get_by_window(win_1).map(|e| e.id), Some(1));

        // 相同配置再次同步 -> None
        assert!(reg.sync_popup(1, cfg_1).is_none());

        // 位置变更 -> Reposition
        let moved_cfg_1 = PopupWindowConfig {
            screen_rect: ScreenRect::new(140, 180, 200, 160),
            ..cfg_1
        };
        assert_eq!(
            reg.sync_popup(1, moved_cfg_1),
            Some(PopupSyncAction::Reposition {
                id: 1,
                window_id: win_1,
                config: moved_cfg_1,
            })
        );

        // 批量差分同步：保留 1（不变）、新增 2 与 3
        let actions = reg.sync_desired(&[(1, moved_cfg_1), (2, cfg_2), (3, cfg_b)]);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], PopupSyncAction::Create { id: 2, .. }));
        assert!(matches!(actions[1], PopupSyncAction::Create { id: 3, .. }));
        assert_eq!(reg.len(), 3);

        // 宿主 owner_a 关闭时一次性级联回收 1 与 2，保留 owner_b 的 3
        let closed = reg.close_for_owner(owner_a);
        assert_eq!(closed.len(), 2);
        assert!(matches!(closed[0], PopupSyncAction::Close { id: 1, .. }));
        assert!(matches!(closed[1], PopupSyncAction::Close { id: 2, .. }));
        assert_eq!(reg.len(), 1);
        assert!(reg.get(3).is_some());

        // 单独关闭 3
        assert!(matches!(
            reg.close_popup(3),
            Some(PopupSyncAction::Close { id: 3, .. })
        ));
        assert!(reg.is_empty());
    }
}
