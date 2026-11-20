//! 渲染管线与构建流水线阶段耗时检视器（`PipelineView`）。
//!
//! 提供阶段描述符（[`PipelineViewDesc`]）、交互状态机（[`PipelineViewState`]，支持阶段选中、
//! 子步骤与日志区展开/折叠、状态徽标与进度驱动）以及 24 态恒定图元数发射器（[`emit_pipeline_view`]）。

use accesskit::Role;
use unbound_a11y::A11yNode;
use unbound_input::{CursorKind, HitRegion};
use unbound_math::{Rect, Vec2};
use unbound_text::GlyphQuad;
use unbound_theme::Token;

use crate::primitive::{focus_ring, glyphs as emit_glyphs, outlined, surface};
use crate::{ControlState, PointerPhase, WidgetOutput};

const LAYER: u8 = 1;

/// 流水线阶段执行状态。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PipelineStageStatus {
    /// 等待执行。
    #[default]
    Pending,
    /// 正在执行。
    Running,
    /// 执行成功。
    Succeeded,
    /// 执行失败。
    Failed,
}

/// 流水线阶段内的单个子步骤。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineSubStep {
    /// 子步骤名称。
    pub name: String,
    /// 子步骤耗时（毫秒）。
    pub duration_ms: f32,
    /// 子步骤执行状态。
    pub status: PipelineStageStatus,
}

impl PipelineSubStep {
    /// 创建子步骤记录。
    #[must_use]
    pub fn new(name: impl Into<String>, duration_ms: f32, status: PipelineStageStatus) -> Self {
        Self {
            name: name.into(),
            duration_ms: duration_ms.max(0.0),
            status,
        }
    }
}

/// 渲染管线或构建流水线的单个阶段。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineStageItem {
    pub name: String,
    pub duration_ms: f32,
    pub active: bool,
    pub has_error: bool,
    /// 阶段状态徽标（与 `active` / `has_error` 协同）。
    pub status: PipelineStageStatus,
    /// 阶段进度（`0.0..=1.0`，若未显式设置则按 `duration_ms` 比例回退）。
    pub progress: Option<f32>,
    /// 可折叠展开的子步骤列表。
    pub substeps: Vec<PipelineSubStep>,
    /// 该阶段关联的诊断日志行。
    pub logs: Vec<String>,
}

impl PipelineStageItem {
    #[must_use]
    pub fn new(name: impl Into<String>, duration_ms: f32, active: bool) -> Self {
        Self {
            name: name.into(),
            duration_ms,
            active,
            has_error: false,
            status: if active {
                PipelineStageStatus::Running
            } else {
                PipelineStageStatus::Pending
            },
            progress: None,
            substeps: Vec::new(),
            logs: Vec::new(),
        }
    }

    /// 设置阶段状态（自动同步 `active` 与 `has_error` 标志）。
    #[must_use]
    pub const fn with_status(mut self, status: PipelineStageStatus) -> Self {
        self.status = status;
        self.active = matches!(status, PipelineStageStatus::Running);
        self.has_error = matches!(status, PipelineStageStatus::Failed);
        self
    }

    /// 设置显式进度（`0.0..=1.0`）。
    #[must_use]
    pub fn with_progress(mut self, progress: f32) -> Self {
        self.progress = Some(progress.clamp(0.0, 1.0));
        self
    }

    /// 附加子步骤列表。
    #[must_use]
    pub fn with_substeps(mut self, substeps: Vec<PipelineSubStep>) -> Self {
        self.substeps = substeps;
        self
    }

    /// 附加日志行。
    #[must_use]
    pub fn with_logs(mut self, logs: Vec<String>) -> Self {
        self.logs = logs;
        self
    }
}

/// 渲染管线阶段视图交互状态机。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PipelineViewState {
    /// 当前阶段列表。
    pub stages: Vec<PipelineStageItem>,
    /// 当前选中的阶段索引。
    pub selected_stage: Option<usize>,
    /// 已展开子步骤的阶段索引列表。
    pub expanded_stages: Vec<usize>,
    /// 是否展开日志区。
    pub logs_expanded: bool,
}

impl PipelineViewState {
    /// 以阶段列表创建交互状态机。
    #[must_use]
    pub fn new(stages: Vec<PipelineStageItem>) -> Self {
        Self {
            stages,
            selected_stage: None,
            expanded_stages: Vec::new(),
            logs_expanded: false,
        }
    }

    /// 选中指定阶段（越界时返回 `false`）。
    pub fn select_stage(&mut self, index: usize) -> bool {
        if index >= self.stages.len() {
            return false;
        }
        self.selected_stage = Some(index);
        true
    }

    /// 清除当前选中阶段。
    pub fn clear_selection(&mut self) {
        self.selected_stage = None;
    }

    /// 切换指定阶段子步骤的展开/折叠状态，返回切换后的展开态。
    pub fn toggle_stage_expanded(&mut self, index: usize) -> bool {
        if index >= self.stages.len() {
            return false;
        }
        if let Some(pos) = self.expanded_stages.iter().position(|&i| i == index) {
            self.expanded_stages.remove(pos);
            false
        } else {
            self.expanded_stages.push(index);
            true
        }
    }

    /// 查询指定阶段是否处于展开态。
    #[must_use]
    pub fn is_stage_expanded(&self, index: usize) -> bool {
        self.expanded_stages.contains(&index)
    }

    /// 切换日志区展开/折叠状态，返回切换后的展开态。
    pub fn toggle_logs(&mut self) -> bool {
        self.logs_expanded = !self.logs_expanded;
        self.logs_expanded
    }

    /// 更新指定阶段的进度与状态。
    pub fn set_stage_progress(
        &mut self,
        index: usize,
        progress: f32,
        status: PipelineStageStatus,
    ) -> bool {
        let Some(stage) = self.stages.get_mut(index) else {
            return false;
        };
        stage.progress = Some(progress.clamp(0.0, 1.0));
        stage.status = status;
        stage.active = matches!(status, PipelineStageStatus::Running);
        stage.has_error = matches!(status, PipelineStageStatus::Failed);
        true
    }

    /// 返回当前选中阶段的日志切片（未选中时返回空切片）。
    #[must_use]
    pub fn active_logs(&self) -> &[String] {
        self.selected_stage
            .and_then(|idx| self.stages.get(idx))
            .map_or(&[], |s| s.logs.as_slice())
    }

    /// 根据点击坐标命中对应阶段卡片并更新选中态，返回命中的阶段索引。
    pub fn handle_click(&mut self, point: Vec2, desc: &PipelineViewDesc) -> Option<usize> {
        for idx in 0..desc.stages.len().min(self.stages.len()) {
            if let Some(card_r) = desc.stage_rect(idx) {
                if card_r.contains_point(point) {
                    self.selected_stage = Some(idx);
                    return Some(idx);
                }
            }
        }
        None
    }

    /// 基于当前状态构造用于发射的 [`PipelineViewDesc`]。
    #[must_use]
    pub fn to_desc(&self, rect: Rect) -> PipelineViewDesc {
        PipelineViewDesc {
            rect,
            stages: self.stages.clone(),
            selected_stage: self.selected_stage,
            expanded_stages: self.expanded_stages.clone(),
            logs_expanded: self.logs_expanded,
            z: 14,
        }
    }
}

/// 渲染管线阶段检视器描述符。
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineViewDesc {
    pub rect: Rect,
    pub stages: Vec<PipelineStageItem>,
    /// 当前选中阶段索引（可选）。
    pub selected_stage: Option<usize>,
    /// 已展开子步骤的阶段索引列表。
    pub expanded_stages: Vec<usize>,
    /// 是否展开诊断日志区。
    pub logs_expanded: bool,
    pub z: u16,
}

impl PipelineViewDesc {
    #[must_use]
    pub fn new(rect: Rect, stages: Vec<PipelineStageItem>) -> Self {
        Self {
            rect,
            stages,
            selected_stage: None,
            expanded_stages: Vec::new(),
            logs_expanded: false,
            z: 14,
        }
    }

    /// 设置当前选中阶段索引。
    #[must_use]
    pub const fn with_selected_stage(mut self, selected: Option<usize>) -> Self {
        self.selected_stage = selected;
        self
    }

    /// 计算第 `idx` 个阶段卡片的布局矩形。
    #[must_use]
    pub fn stage_rect(&self, idx: usize) -> Option<Rect> {
        if idx >= self.stages.len() {
            return None;
        }
        let r = self.rect;
        let count = self.stages.len().max(1) as f32;
        let gap = 8.0;
        let stage_w = ((r.size.x - 24.0 - (count - 1.0) * gap) / count).max(32.0);
        let sx = r.origin.x + 12.0 + idx as f32 * (stage_w + gap);
        Some(Rect::new(
            sx,
            r.origin.y + 12.0,
            stage_w,
            (r.size.y - 24.0).max(28.0),
        ))
    }
}

/// 发射管线阶段视图（保持 24 态恒定图元数）。
#[must_use]
pub fn emit_pipeline_view(
    node: u64,
    desc: &PipelineViewDesc,
    control: ControlState,
    glyphs: &[GlyphQuad],
) -> WidgetOutput {
    let mut primitives = Vec::with_capacity(2 + desc.stages.len() * 2 + glyphs.len());
    let mut payloads = Vec::with_capacity(glyphs.len());
    let r = desc.rect;
    let z = desc.z;

    let outer_border = if !control.disabled && control.phase == PointerPhase::Hovered {
        Token::OUTLINE_STRONG
    } else {
        Token::CARD_BORDER
    };
    // 外层容器是卡片，圆角对齐 WinUI `OverlayCornerRadius = 8`。
    primitives.push(outlined(r, 8.0, Token::SURFACE, outer_border, 1.0, 0.0, z));

    for (idx, stage) in desc.stages.iter().enumerate() {
        let card_r = desc.stage_rect(idx).unwrap_or(r);
        let is_selected = desc.selected_stage == Some(idx);
        let (fill, border) = if stage.has_error || stage.status == PipelineStageStatus::Failed {
            (Token::ERROR_SOFT, Token::ERROR)
        } else if is_selected {
            (Token::SELECTION, Token::PRIMARY)
        } else if stage.active || stage.status == PipelineStageStatus::Running {
            (Token::PRIMARY_SOFT, Token::PRIMARY)
        } else if stage.status == PipelineStageStatus::Succeeded {
            (Token::SUCCESS_SOFT, Token::SUCCESS)
        } else {
            (Token::SURFACE_RAISED, Token::OUTLINE)
        };
        // 阶段卡片圆角对齐 WinUI `ControlCornerRadius = 4`，与按钮/输入框同档。
        primitives.push(outlined(card_r, 4.0, fill, border, 1.0, 0.0, z + 1));

        // 底部 GPU 耗时占比 / 显式进度条。
        let ratio = stage
            .progress
            .map_or_else(|| (stage.duration_ms / 4.0).clamp(0.08, 1.0), |p| p.clamp(0.04, 1.0));
        let bar_r = Rect::new(
            card_r.origin.x + 6.0,
            card_r.max().y - 10.0,
            (card_r.size.x - 12.0) * ratio,
            4.0,
        );
        let bar_tok = if stage.has_error || stage.status == PipelineStageStatus::Failed {
            Token::ERROR
        } else if stage.status == PipelineStageStatus::Succeeded {
            Token::SUCCESS
        } else {
            Token::PRIMARY
        };
        primitives.push(surface(bar_r, 2.0, bar_tok, z + 2));
    }

    emit_glyphs(
        glyphs,
        Token::TEXT_PRIMARY,
        z + 3,
        &mut primitives,
        &mut payloads,
    );
    primitives.push(focus_ring(
        r.inflate(2.0),
        10.0,
        Token::FOCUS_RING,
        2.0,
        z + 4,
        control.focused && !control.disabled,
    ));

    WidgetOutput {
        primitives,
        payloads,
        hit: (!control.disabled)
            .then(|| HitRegion::new(node, LAYER, u32::from(z), r).with_cursor(CursorKind::Pointer)),
        a11y: Some(
            A11yNode::new(Role::Group, "Pipeline View", r)
                .expanded(desc.logs_expanded || !desc.expanded_stages.is_empty())
                .disabled(control.disabled),
        ),
        focusable: !control.disabled,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unbound_headless::{FrameBuilder, Harness, NodeKind};

    #[test]
    fn pipeline_view_state_selection_expand_logs_and_progress() {
        let stages = vec![
            PipelineStageItem::new("Geometry", 1.8, false)
                .with_status(PipelineStageStatus::Succeeded)
                .with_substeps(vec![PipelineSubStep::new(
                    "Meshlet Cull",
                    0.6,
                    PipelineStageStatus::Succeeded,
                )])
                .with_logs(vec!["culled 4096 meshlets".to_owned()]),
            PipelineStageItem::new("Lighting", 2.4, true)
                .with_progress(0.5)
                .with_logs(vec!["tile light binning".to_owned()]),
        ];
        let mut st = PipelineViewState::new(stages);
        let rect = Rect::new(20.0, 20.0, 480.0, 120.0);
        let desc = st.to_desc(rect);

        // 点击第二个阶段卡片命中并选中。
        let second_center = desc.stage_rect(1).unwrap().center();
        assert_eq!(st.handle_click(second_center, &desc), Some(1));
        assert_eq!(st.selected_stage, Some(1));
        assert_eq!(st.active_logs(), &["tile light binning"]);

        // 展开/折叠子步骤与日志区。
        assert!(st.toggle_stage_expanded(0));
        assert!(st.is_stage_expanded(0));
        assert!(!st.toggle_stage_expanded(0));
        assert!(st.toggle_logs());
        assert!(st.logs_expanded);

        // 更新进度与失败状态。
        assert!(st.set_stage_progress(1, 1.0, PipelineStageStatus::Failed));
        assert!(st.stages[1].has_error);
        assert!(!st.stages[1].active);
    }

    #[test]
    fn pipeline_view_empty_stages_and_headless_geometry() {
        let rect = Rect::new(20.0, 20.0, 400.0, 96.0);
        let mut st = PipelineViewState::new(Vec::new());
        assert!(!st.select_stage(0));
        assert!(st.active_logs().is_empty());

        let desc = st.to_desc(rect);
        let out = emit_pipeline_view(1, &desc, ControlState::default(), &[]);
        assert_eq!(out.primitives.len(), 2);

        let viewport = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let mut frame = FrameBuilder::new(viewport);
        frame.open(NodeKind::Container, viewport, "Root");
        frame.open(
            NodeKind::Interactive {
                min_size: NodeKind::DEFAULT_MIN_SIZE,
                occluded: false,
            },
            rect,
            "PipelineView",
        );
        frame.close();
        frame.close();
        assert!(Harness::assert_clean(&frame.build()).is_clean());
    }
}
