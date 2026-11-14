//! 指针捕获与点击/拖拽状态机（`docs` §6.1）。
//!
//! 一旦开始拖拽，指针即被**捕获**：后续所有移动与释放事件都会路由到
//! 被捕获的节点，即使光标离开窗口或划过其他无关控件亦然。这一条规则
//! 保证了滚动条滑块、框选与画布平移在快速移动下的稳定表现，也正是本模块
//! 统一管理拖拽决策而非交由各控件自行处理的原因。
//!
//!
//!
//! 时间以 `f64` 秒数显式传入，而非直接读取系统时钟，从而使整个状态机
//! 具有完全的确定性，并可在无头测试框架中重放。

#![forbid(unsafe_op_in_unsafe_fn)]

use unbound_math::Vec2;

/// 产生该事件的鼠标按键。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

/// 当前处于按住状态的按键掩码。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl Buttons {
    fn set(&mut self, button: PointerButton, pressed: bool) {
        match button {
            PointerButton::Left => self.left = pressed,
            PointerButton::Right => self.right = pressed,
            PointerButton::Middle => self.middle = pressed,
        }
    }
}

/// 输入层向外发送的所有指针事件。
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PointerEvent {
    Down {
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
        click_count: u8,
    },
    Move {
        position: Vec2,
        delta: Vec2,
        target: Option<u64>,
    },
    DragStart {
        target: u64,
        position: Vec2,
    },
    Drag {
        target: u64,
        position: Vec2,
        delta: Vec2,
    },
    Up {
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
    },
    Click {
        target: u64,
        position: Vec2,
        count: u8,
    },
    DoubleClick {
        target: u64,
        position: Vec2,
    },
    /// 捕获被强制收回（窗口失焦、按下 Escape 或目标节点被移除）。
    CaptureLost {
        target: u64,
    },
}

impl PointerEvent {
    /// 事件关联的目标节点（若有）。
    #[must_use]
    pub fn target(&self) -> Option<u64> {
        match self {
            Self::Down { target, .. } | Self::Move { target, .. } | Self::Up { target, .. } => {
                *target
            }
            Self::DragStart { target, .. }
            | Self::Drag { target, .. }
            | Self::Click { target, .. }
            | Self::DoubleClick { target, .. }
            | Self::CaptureLost { target } => Some(*target),
        }
    }
}

/// 时间与距离阈值配置。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PointerConfig {
    /// 按下后移动多少距离才判定为开始拖拽。
    pub drag_slop: f32,
    /// 双击两次点击之间的最大时间间隔（单位：秒）。
    pub double_click_window: f64,
    /// 双击两次点击之间的最大允许位移。
    pub double_click_slop: f32,
}

impl Default for PointerConfig {
    fn default() -> Self {
        Self {
            drag_slop: 3.0,
            double_click_window: 0.5,
            double_click_slop: 4.0,
        }
    }
}

/// 单指针状态机。
#[derive(Clone, Debug)]
pub struct PointerState {
    config: PointerConfig,
    position: Vec2,
    down_position: Vec2,
    pressed: Option<u64>,
    /// 启动当前交互的按键。
    ///
    /// 只有该按键的释放才能结束当前交互：在主键按住期间释放次键
    /// 绝不能中止拖拽。
    press_button: Option<PointerButton>,
    capture: Option<u64>,
    dragging: bool,
    buttons: Buttons,
    click_count: u8,
    last_click_time: f64,
    last_click_position: Vec2,
    last_click_target: Option<u64>,
}

impl PointerState {
    #[must_use]
    pub fn new(config: PointerConfig) -> Self {
        Self {
            config,
            position: Vec2::ZERO,
            down_position: Vec2::ZERO,
            pressed: None,
            press_button: None,
            capture: None,
            dragging: false,
            buttons: Buttons::default(),
            click_count: 0,
            last_click_time: f64::NEG_INFINITY,
            last_click_position: Vec2::ZERO,
            last_click_target: None,
        }
    }

    #[must_use]
    pub fn position(&self) -> Vec2 {
        self.position
    }

    #[must_use]
    pub fn capture(&self) -> Option<u64> {
        self.capture
    }

    #[must_use]
    pub fn pressed(&self) -> Option<u64> {
        self.pressed
    }

    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    #[must_use]
    pub fn buttons(&self) -> Buttons {
        self.buttons
    }

    /// 当前按下周期累计的点击次数。
    #[must_use]
    pub fn click_count(&self) -> u8 {
        self.click_count
    }

    /// 在 `target` 上按下按键。
    pub fn on_down(
        &mut self,
        target: Option<u64>,
        position: Vec2,
        button: PointerButton,
        now: f64,
    ) -> Option<PointerEvent> {
        self.position = position;
        self.buttons.set(button, true);
        if button == PointerButton::Right {
            // 次键按下并在释放时打开上下文菜单。它与其他按键一样被捕获，
            // 但不得干扰双击计时器：先右键再左键属于两个不同的手势。
            // 若主键拖拽已在进行中，则保留原捕获，次键仅上报自身事件。
            //
            //
            let primary_active = matches!(
                self.press_button,
                Some(PointerButton::Left | PointerButton::Middle)
            );
            if !primary_active {
                self.down_position = position;
                self.pressed = target;
                self.press_button = Some(button);
                self.capture = target;
                self.dragging = false;
            }
            return Some(PointerEvent::Down {
                target,
                position,
                button,
                click_count: 1,
            });
        }
        let repeat = target.is_some()
            && target == self.last_click_target
            && now - self.last_click_time <= self.config.double_click_window
            && (position - self.last_click_position).length() <= self.config.double_click_slop;
        self.click_count = if repeat {
            (self.click_count + 1).min(3)
        } else {
            1
        };
        self.last_click_time = now;
        self.last_click_position = position;
        self.last_click_target = target;
        self.down_position = position;
        self.pressed = target;
        self.press_button = Some(button);
        self.capture = target;
        self.dragging = false;
        Some(PointerEvent::Down {
            target,
            position,
            button,
            click_count: self.click_count,
        })
    }

    /// 指针移动。每次最多返回一个事件：发生状态转换的那一帧上报
    /// `DragStart` 而非 `Drag`，使消费方不会同时收到两者。
    pub fn on_move(&mut self, position: Vec2) -> Option<PointerEvent> {
        let delta = position - self.position;
        self.position = position;
        if let Some(target) = self.pressed {
            if !self.dragging {
                // 只有拥有拖拽手势的按键才能跨越移动阈值：
                // 次键在释放前始终保持按压状态。
                let draggable = matches!(
                    self.press_button,
                    Some(PointerButton::Left | PointerButton::Middle)
                );
                if draggable && (position - self.down_position).length() >= self.config.drag_slop {
                    self.dragging = true;
                    return Some(PointerEvent::DragStart { target, position });
                }
                return Some(PointerEvent::Move {
                    position,
                    delta,
                    target: Some(target),
                });
            }
            if self.buttons.left || self.buttons.middle {
                return Some(PointerEvent::Drag {
                    target,
                    position,
                    delta,
                });
            }
        }
        Some(PointerEvent::Move {
            position,
            delta,
            target: self.capture,
        })
    }

    /// 指针释放。
    ///
    /// 返回 `Up` 事件；若该按压从未演变为拖拽，则紧随其后返回 `Click`
    /// （若为连续第二次点击，则附带 `DoubleClick`）。
    pub fn on_up(&mut self, position: Vec2, button: PointerButton, now: f64) -> Vec<PointerEvent> {
        self.position = position;
        self.buttons.set(button, false);
        // 只有启动当前交互的按键才能结束它。在主键按住期间释放
        // **次键**绝不能结束拖拽：用户此时仍按住左键在拖拽。
        //
        if self.press_button != Some(button) {
            let events = vec![PointerEvent::Up {
                target: self.pressed,
                position,
                button,
            }];
            // 次键释放不构成点击，且不得触动双击计时器，
            // 否则先右键再左键会被误判为双击。
            //
            return events;
        }
        let target = self.pressed;
        let dragged = self.dragging;
        self.pressed = None;
        self.press_button = None;
        self.capture = None;
        self.dragging = false;

        let mut events = vec![PointerEvent::Up {
            target,
            position,
            button,
        }];
        if button == PointerButton::Left && !dragged {
            if let Some(target) = target {
                events.push(PointerEvent::Click {
                    target,
                    position,
                    count: self.click_count,
                });
                if self.click_count == 2 {
                    events.push(PointerEvent::DoubleClick { target, position });
                }
            }
        } else if !dragged {
            self.last_click_time = now;
        }
        events
    }

    /// 中止当前交互（窗口失焦、按下 Escape 或目标节点被移除）。
    pub fn cancel(&mut self) -> Option<PointerEvent> {
        let target = self.capture.or(self.pressed)?;
        self.pressed = None;
        self.press_button = None;
        self.capture = None;
        self.dragging = false;
        self.buttons = Buttons::default();
        Some(PointerEvent::CaptureLost { target })
    }

    /// 重置待配对的点击计数。
    ///
    /// 当释放位置落在原按下目标之外、路由器扣留点击事件时调用：
    /// 既然未派发点击，下一次按下必须从 1 重新计数，而非触发幽灵双击。
    ///
    pub fn reset_click_sequence(&mut self) {
        self.click_count = 0;
        self.last_click_time = f64::NEG_INFINITY;
        self.last_click_target = None;
    }

    /// 在未发生按下的情况下直接接管捕获（例如滚动条滑块抓取）。
    ///
    /// 原捕获持有者会被逐出并收到通知：若捕获悄无声息地消失而未触发
    /// `CaptureLost`，原持有者将卡死在拖拽状态中。
    ///
    pub fn grab(&mut self, target: u64) -> Option<PointerEvent> {
        let previous = self.capture.replace(target)?;
        if previous == target {
            return None;
        }
        Some(PointerEvent::CaptureLost { target: previous })
    }

    /// 释放捕获但保留按键状态。
    ///
    /// 通知原持有者其已失去捕获。
    pub fn release_capture(&mut self) -> Option<PointerEvent> {
        let previous = self.capture.take()?;
        Some(PointerEvent::CaptureLost { target: previous })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PointerState {
        PointerState::new(PointerConfig::default())
    }

    #[test]
    fn a_press_and_release_without_movement_is_a_click() {
        let mut p = state();
        let down = p.on_down(Some(7), Vec2::new(10.0, 10.0), PointerButton::Left, 0.0);
        assert!(matches!(
            down,
            Some(PointerEvent::Down {
                target: Some(7),
                click_count: 1,
                ..
            })
        ));
        let events = p.on_up(Vec2::new(10.0, 10.0), PointerButton::Left, 0.1);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], PointerEvent::Up { .. }));
        assert!(matches!(
            events[1],
            PointerEvent::Click {
                target: 7,
                count: 1,
                ..
            }
        ));
        assert_eq!(p.capture(), None);
    }

    #[test]
    fn movement_past_the_slop_starts_a_drag_and_suppresses_the_click() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let start = p.on_move(Vec2::new(5.0, 0.0)).unwrap();
        assert!(matches!(start, PointerEvent::DragStart { target: 3, .. }));
        let drag = p.on_move(Vec2::new(9.0, 0.0)).unwrap();
        assert!(matches!(
            drag,
            PointerEvent::Drag {
                target: 3,
                delta,
                ..
            } if (delta.x - 4.0).abs() < 1e-6
        ));
        let events = p.on_up(Vec2::new(9.0, 0.0), PointerButton::Left, 0.2);
        assert_eq!(events.len(), 1, "a drag must not synthesise a click");
        assert!(matches!(events[0], PointerEvent::Up { .. }));
    }

    #[test]
    fn sub_slop_movement_stays_a_move() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let event = p.on_move(Vec2::new(1.0, 0.0)).unwrap();
        assert!(matches!(
            event,
            PointerEvent::Move {
                target: Some(3),
                ..
            }
        ));
        assert!(!p.is_dragging());
    }

    #[test]
    fn the_capture_keeps_routing_to_the_original_target() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        p.on_move(Vec2::new(50.0, 0.0));
        assert_eq!(p.capture(), Some(1));
        let event = p.on_move(Vec2::new(500.0, 500.0)).unwrap();
        assert_eq!(
            event.target(),
            Some(1),
            "capture must survive leaving the node"
        );
    }

    #[test]
    fn a_double_click_is_detected_within_the_window() {
        let mut p = state();
        p.on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.1);
        let down = p
            .on_down(Some(4), Vec2::new(21.0, 20.0), PointerButton::Left, 0.2)
            .unwrap();
        assert!(matches!(down, PointerEvent::Down { click_count: 2, .. }));
        let events = p.on_up(Vec2::new(21.0, 20.0), PointerButton::Left, 0.25);
        assert!(events
            .iter()
            .any(|e| matches!(e, PointerEvent::DoubleClick { target: 4, .. })));
    }

    #[test]
    fn a_slow_second_click_is_a_fresh_single_click() {
        let mut p = state();
        p.on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.05);
        let down = p
            .on_down(Some(4), Vec2::new(20.0, 20.0), PointerButton::Left, 2.0)
            .unwrap();
        assert!(matches!(down, PointerEvent::Down { click_count: 1, .. }));
    }

    #[test]
    fn clicking_a_different_target_resets_the_count() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(20.0, 20.0), PointerButton::Left, 0.0);
        p.on_up(Vec2::new(20.0, 20.0), PointerButton::Left, 0.05);
        let down = p
            .on_down(Some(2), Vec2::new(20.0, 20.0), PointerButton::Left, 0.1)
            .unwrap();
        assert!(matches!(down, PointerEvent::Down { click_count: 1, .. }));
    }

    #[test]
    fn the_right_button_never_starts_a_drag() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Right, 0.0);
        let event = p.on_move(Vec2::new(100.0, 0.0)).unwrap();
        assert!(matches!(event, PointerEvent::Move { .. }));
        assert!(!p.is_dragging());
    }

    /// 次键按下会被捕获以便释放时拥有目标，但它不产生点击：
    /// 运行时将其转换为上下文菜单请求。
    #[test]
    fn a_secondary_press_is_captured_but_is_not_a_click() {
        let mut p = state();
        let down = p
            .on_down(Some(4), Vec2::new(10.0, 10.0), PointerButton::Right, 0.0)
            .unwrap();
        assert!(matches!(
            down,
            PointerEvent::Down {
                button: PointerButton::Right,
                ..
            }
        ));
        assert_eq!(p.capture(), Some(4), "the release needs a target");
        assert!(!p.is_dragging());
        let events = p.on_up(Vec2::new(10.0, 10.0), PointerButton::Right, 0.05);
        assert!(matches!(
            events[0],
            PointerEvent::Up {
                target: Some(4),
                button: PointerButton::Right,
                ..
            }
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PointerEvent::Click { .. })),
            "a right click is a menu request, not an activation"
        );
    }

    /// 中键与主键一样拥有拖拽权（用于画布平移），但永不合成点击事件。
    ///
    #[test]
    fn the_middle_button_drags_without_clicking() {
        let mut p = state();
        p.on_down(Some(2), Vec2::new(0.0, 0.0), PointerButton::Middle, 0.0);
        let start = p.on_move(Vec2::new(10.0, 0.0)).unwrap();
        assert!(matches!(start, PointerEvent::DragStart { target: 2, .. }));
        let drag = p.on_move(Vec2::new(20.0, 0.0)).unwrap();
        assert!(matches!(drag, PointerEvent::Drag { target: 2, .. }));
        let events = p.on_up(Vec2::new(20.0, 0.0), PointerButton::Middle, 0.1);
        assert_eq!(events.len(), 1);
        assert!(!events
            .iter()
            .any(|event| matches!(event, PointerEvent::Click { .. })));
    }

    #[test]
    fn cancel_reports_the_capture_and_clears_state() {
        let mut p = state();
        p.on_down(Some(5), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        p.on_move(Vec2::new(20.0, 0.0));
        let event = p.cancel().unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 5 });
        assert_eq!(p.capture(), None);
        assert!(!p.is_dragging());
        assert!(p.cancel().is_none(), "second cancel is a no-op");
    }

    #[test]
    fn a_click_outside_any_target_still_reports_up() {
        let mut p = state();
        p.on_down(None, Vec2::new(1.0, 1.0), PointerButton::Left, 0.0);
        let events = p.on_up(Vec2::new(1.0, 1.0), PointerButton::Left, 0.1);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], PointerEvent::Up { target: None, .. }));
    }

    /// 释放次键绝不能中止主键拖拽。
    ///
    /// 用户仍按住左键并继续拖拽：交互持续进行，捕获不会被释放。
    ///
    #[test]
    fn releasing_the_secondary_button_does_not_abort_the_drag() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        p.on_move(Vec2::new(30.0, 0.0));
        assert!(p.is_dragging());
        // 左键按住期间，右键完成一次按下与释放。
        p.on_down(Some(3), Vec2::new(30.0, 0.0), PointerButton::Right, 0.1);
        let events = p.on_up(Vec2::new(30.0, 0.0), PointerButton::Right, 0.2);
        assert!(matches!(
            events[0],
            PointerEvent::Up {
                button: PointerButton::Right,
                ..
            }
        ));
        assert!(p.is_dragging(), "the drag must survive a secondary release");
        assert_eq!(p.capture(), Some(3), "the capture must survive too");
        // 拖拽依然持续上报 Drag 事件。
        let event = p.on_move(Vec2::new(60.0, 0.0)).unwrap();
        assert!(matches!(event, PointerEvent::Drag { .. }));
    }

    /// 次键释放绝不能触动双击计时器。
    ///
    /// 否则在同一目标上先右键再左键会被误判为双击。
    ///
    #[test]
    fn a_secondary_release_does_not_create_a_double_click() {
        let mut p = state();
        p.on_down(Some(3), Vec2::new(10.0, 10.0), PointerButton::Right, 0.0);
        p.on_up(Vec2::new(10.0, 10.0), PointerButton::Right, 0.05);
        // 紧随其后的左键点击必须是单击，而非双击。
        p.on_down(Some(3), Vec2::new(10.0, 10.0), PointerButton::Left, 0.1);
        let events = p.on_up(Vec2::new(10.0, 10.0), PointerButton::Left, 0.15);
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, PointerEvent::DoubleClick { .. })),
            "a right-click must not arm the double-click: {events:?}"
        );
    }

    /// 抢占或释放捕获时必须通知前一个持有者。
    ///
    /// 若捕获消失却未收到 `CaptureLost`，原持有者会卡死在拖拽状态中。
    ///
    #[test]
    fn stealing_the_capture_reports_the_loss() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let event = p.grab(9).unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 1 });
        assert_eq!(p.capture(), Some(9));
    }

    #[test]
    fn releasing_the_capture_reports_the_loss() {
        let mut p = state();
        p.on_down(Some(1), Vec2::new(0.0, 0.0), PointerButton::Left, 0.0);
        let event = p.release_capture().unwrap();
        assert_eq!(event, PointerEvent::CaptureLost { target: 1 });
        assert_eq!(p.capture(), None);
        assert!(p.release_capture().is_none(), "a second release is a no-op");
    }
}
