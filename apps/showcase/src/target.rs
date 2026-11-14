//! 展厅外壳的稳定控件标识。
//!
//! 渲染器从不按帧分配：每个交互控件都有一个由其角色和页面位置派生的确定性 `u64` 键。
//! 同一个键贯穿 `unbound-input`（命中区域）、`unbound-a11y`（语义）和事件 reducer，
//! 从而使整个外壳拥有唯一统一的标识命名空间。
//!
//!

/// 展厅可激活的所有目标。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// 折叠 / 展开导航栏。
    NavToggle,
    /// 切换为浅色（`0`）或深色（`1`）主题。
    Theme(u8),
    /// 选择 UI 缩放档位（`0` = 100%，`3` = 200%）。
    Zoom(u8),
    /// 切换静态像素对齐。
    Snap,
    /// 切换性能 HUD。
    Hud,
    /// 聚焦全局搜索框。
    Search,
    /// 跳转到搜索结果。
    SearchResult(u8),
    /// 导航到指定页面。
    NavPage(u8),
    /// 展开 / 折叠示例卡片的代码面板。
    Code(u8, u8),
    /// 激活旋钮（开关 / 复选框行）。
    Knob(u8, u8, u8),
    /// 选择分段控件选项。
    KnobOption(u8, u8, u8, u8),
    /// 激活示例实时舞台中的某个部件。
    Demo(u8, u8, u16),
    /// 页脚快捷方式：`0` 遥测，`1` 设置。
    Footer(u8),
    /// 最小化原生窗口。
    WindowMinimize,
    /// 切换原生窗口的最大化状态。
    WindowMaximize,
    /// 关闭原生窗口。
    WindowClose,
    /// 拖拽页面的垂直滚动条。
    ContentScrollbar,
    /// 拖拽导航栏的垂直滚动条。
    NavScrollbar,
    /// 选择 FPS 上限预设（`0` = 60，`1` = 120，`2` = 144，`3` = 240，
    /// `4` = 无限制）。
    FpsPreset(u8),
    /// 聚焦自定义 FPS 上限输入框。
    FpsInput,
}

const TAG_NAV_TOGGLE: u64 = 1;
const TAG_THEME: u64 = 2;
const TAG_ZOOM: u64 = 3;
const TAG_SNAP: u64 = 4;
const TAG_HUD: u64 = 5;
const TAG_SEARCH: u64 = 6;
const TAG_SEARCH_RESULT: u64 = 7;
const TAG_NAV_PAGE: u64 = 8;
const TAG_CODE: u64 = 9;
const TAG_KNOB: u64 = 10;
const TAG_KNOB_OPTION: u64 = 11;
const TAG_DEMO: u64 = 12;
const TAG_FOOTER: u64 = 13;
const TAG_WINDOW_MINIMIZE: u64 = 14;
const TAG_WINDOW_MAXIMIZE: u64 = 15;
const TAG_WINDOW_CLOSE: u64 = 16;
const TAG_CONTENT_SCROLLBAR: u64 = 17;
const TAG_FPS_PRESET: u64 = 18;
const TAG_FPS_INPUT: u64 = 19;
const TAG_NAV_SCROLLBAR: u64 = 20;

/// 展厅的语义根节点。
pub const ROOT: u64 = 1;

/// 判断某标识是否属于示例的实时舞台。
///
/// 演示属于定制绘制：与标准控件不同，它可能在悬停时显露新控件
/// （如表格行高亮、对话框阴影），因此允许改变场景的形状。
///
#[must_use]
pub const fn is_demo(node: u64) -> bool {
    node >> 56 == TAG_DEMO
}

const fn tag(value: u64) -> u64 {
    value << 56
}

/// 将目标编码进共享标识命名空间。
#[must_use]
pub const fn id(target: Target) -> u64 {
    match target {
        Target::NavToggle => tag(TAG_NAV_TOGGLE),
        Target::Theme(bit) => tag(TAG_THEME) | bit as u64,
        Target::Zoom(step) => tag(TAG_ZOOM) | step as u64,
        Target::Snap => tag(TAG_SNAP),
        Target::Hud => tag(TAG_HUD),
        Target::Search => tag(TAG_SEARCH),
        Target::SearchResult(page) => tag(TAG_SEARCH_RESULT) | page as u64,
        Target::NavPage(page) => tag(TAG_NAV_PAGE) | page as u64,
        Target::Code(page, card) => tag(TAG_CODE) | ((page as u64) << 8) | card as u64,
        Target::Knob(page, card, knob) => {
            tag(TAG_KNOB) | ((page as u64) << 16) | ((card as u64) << 8) | knob as u64
        }
        Target::KnobOption(page, card, knob, option) => {
            tag(TAG_KNOB_OPTION)
                | ((page as u64) << 24)
                | ((card as u64) << 16)
                | ((knob as u64) << 8)
                | option as u64
        }
        Target::Demo(page, card, part) => {
            tag(TAG_DEMO) | ((page as u64) << 24) | ((card as u64) << 16) | part as u64
        }
        Target::Footer(index) => tag(TAG_FOOTER) | index as u64,
        Target::WindowMinimize => tag(TAG_WINDOW_MINIMIZE),
        Target::WindowMaximize => tag(TAG_WINDOW_MAXIMIZE),
        Target::WindowClose => tag(TAG_WINDOW_CLOSE),
        Target::ContentScrollbar => tag(TAG_CONTENT_SCROLLBAR),
        Target::NavScrollbar => tag(TAG_NAV_SCROLLBAR),
        Target::FpsPreset(index) => tag(TAG_FPS_PRESET) | index as u64,
        Target::FpsInput => tag(TAG_FPS_INPUT),
    }
}

/// 解码由 [`id`] 生成的标识。
#[must_use]
pub const fn decode(value: u64) -> Option<Target> {
    let tag = value >> 56;
    let low = value & 0x00ff_ffff_ffff_ffff;
    Some(match tag {
        TAG_NAV_TOGGLE => Target::NavToggle,
        TAG_THEME => Target::Theme((low & 0xff) as u8),
        TAG_ZOOM => Target::Zoom((low & 0xff) as u8),
        TAG_SNAP => Target::Snap,
        TAG_HUD => Target::Hud,
        TAG_SEARCH => Target::Search,
        TAG_SEARCH_RESULT => Target::SearchResult((low & 0xff) as u8),
        TAG_NAV_PAGE => Target::NavPage((low & 0xff) as u8),
        TAG_CODE => Target::Code(((low >> 8) & 0xff) as u8, (low & 0xff) as u8),
        TAG_KNOB => Target::Knob(
            ((low >> 16) & 0xff) as u8,
            ((low >> 8) & 0xff) as u8,
            (low & 0xff) as u8,
        ),
        TAG_KNOB_OPTION => Target::KnobOption(
            ((low >> 24) & 0xff) as u8,
            ((low >> 16) & 0xff) as u8,
            ((low >> 8) & 0xff) as u8,
            (low & 0xff) as u8,
        ),
        TAG_DEMO => Target::Demo(
            ((low >> 24) & 0xff) as u8,
            ((low >> 16) & 0xff) as u8,
            (low & 0xffff) as u16,
        ),
        TAG_FOOTER => Target::Footer((low & 0xff) as u8),
        TAG_WINDOW_MINIMIZE => Target::WindowMinimize,
        TAG_WINDOW_MAXIMIZE => Target::WindowMaximize,
        TAG_WINDOW_CLOSE => Target::WindowClose,
        TAG_CONTENT_SCROLLBAR => Target::ContentScrollbar,
        TAG_NAV_SCROLLBAR => Target::NavScrollbar,
        TAG_FPS_PRESET => Target::FpsPreset((low & 0xff) as u8),
        TAG_FPS_INPUT => Target::FpsInput,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_round_trips_through_its_identity() {
        let targets = [
            Target::NavToggle,
            Target::Theme(1),
            Target::Zoom(3),
            Target::Snap,
            Target::Hud,
            Target::Search,
            Target::SearchResult(7),
            Target::NavPage(12),
            Target::Code(3, 2),
            Target::Knob(4, 1, 2),
            Target::KnobOption(4, 1, 2, 3),
            Target::Demo(5, 2, 500),
            Target::Footer(1),
            Target::WindowMinimize,
            Target::WindowMaximize,
            Target::WindowClose,
            Target::ContentScrollbar,
            Target::NavScrollbar,
            Target::FpsPreset(4),
            Target::FpsInput,
        ];
        for target in targets {
            assert_eq!(decode(id(target)), Some(target), "{target:?}");
            assert_ne!(id(target), ROOT);
        }
    }

    #[test]
    fn ids_are_unique_across_fields() {
        assert_ne!(id(Target::Code(1, 2)), id(Target::Code(2, 1)));
        assert_ne!(id(Target::NavPage(2)), id(Target::NavPage(3)));
        assert_ne!(
            id(Target::Knob(1, 2, 3)),
            id(Target::KnobOption(1, 2, 3, 0))
        );
    }
}
