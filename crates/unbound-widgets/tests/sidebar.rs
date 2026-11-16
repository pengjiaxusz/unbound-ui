//! 整栏侧边栏的公开契约：跨状态形态恒定、命中切分、排版边界与折叠模式。
//!
//! 这些断言只走 `unbound-widgets` 的公开 API，因此同时也是“展厅之外的下游
//! 能否只靠公开面驱动侧边栏”的回归守卫。

use accesskit::Role;
use unbound_math::Rect;
use unbound_widgets::{
    emit_sidebar, ControlState, PointerPhase, ShellIcon, SidebarDesc, SidebarEntry, SidebarRowKind,
    SidebarSection, SIDEBAR_COLLAPSED_WIDTH, SIDEBAR_PADDING, SIDEBAR_WIDTH,
};

fn demo() -> SidebarDesc {
    SidebarDesc::new(
        Rect::new(0.0, 0.0, SIDEBAR_WIDTH, 420.0),
        vec![
            SidebarSection::new("Workspace")
                .with_entry(SidebarEntry::new(1, "Overview", ShellIcon::Grid))
                .with_entry(SidebarEntry::new(2, "Sessions", ShellIcon::Bolt).badge("3")),
            SidebarSection::new("System").with_entry(SidebarEntry::new(
                3,
                "Settings",
                ShellIcon::Settings,
            )),
        ],
    )
    .active(2)
    .footer(vec![SidebarEntry::new(9, "Help", ShellIcon::Search)])
}

#[test]
fn keeps_constant_primitive_count_across_states() {
    let desc = demo();
    let baseline = emit_sidebar(7, &desc, ControlState::default(), &[]);
    let count = baseline.primitives.len();
    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    let state = ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    };
                    let out = emit_sidebar(7, &desc, state, &[]);
                    assert_eq!(out.primitives.len(), count, "state {state:?}");
                }
            }
        }
    }
}

#[test]
fn emits_one_sub_hit_per_entry() {
    let desc = demo();
    let out = emit_sidebar(7, &desc, ControlState::default(), &[]);
    // 两个分组共 3 个条目 + 1 个页脚条目
    assert_eq!(out.sub_hits.len(), 4);
    assert_eq!(out.sub_a11y.len(), 4);
    assert!(out.hit.is_some());
    assert_eq!(out.a11y.map(|node| node.role), Some(Role::Navigation));
}

#[test]
fn hover_and_selection_can_land_on_different_rows() {
    let desc = demo().hovered(1);
    let out = emit_sidebar(7, &desc, ControlState::default(), &[]);
    let selected: Vec<bool> = out.sub_a11y.iter().map(|(_, node)| node.selected).collect();
    assert_eq!(selected, vec![false, true, false, false]);
}

#[test]
fn rows_are_ordered_and_stay_inside_the_panel() {
    let desc = demo();
    let rows = desc.rows();
    assert!(!rows.is_empty());
    for row in &rows {
        assert!(row.rect.origin.x >= desc.rect.origin.x);
        assert!(row.rect.max().x <= desc.rect.max().x + 0.01);
        assert!(row.rect.origin.y >= desc.rect.origin.y);
        assert!(row.rect.max().y <= desc.rect.max().y + 0.01);
    }
    for pair in rows.windows(2) {
        assert!(
            pair[0].rect.max().y <= pair[1].rect.origin.y + 0.01,
            "rows must not overlap: {:?} / {:?}",
            pair[0].rect,
            pair[1].rect
        );
    }
}

#[test]
fn collapsed_mode_drops_section_titles_but_keeps_hit_targets() {
    let desc = demo().collapsed(true);
    assert!((desc.panel_width() - SIDEBAR_COLLAPSED_WIDTH).abs() < 0.01);
    assert!(
        desc.rows()
            .iter()
            .all(|row| !matches!(row.kind, SidebarRowKind::Section { .. })),
        "collapsed sidebars must drop section headers"
    );
    let out = emit_sidebar(7, &desc, ControlState::default(), &[]);
    assert_eq!(out.sub_hits.len(), 4);
}

#[test]
fn rows_skip_everything_below_the_footer() {
    let mut desc = demo();
    desc.rect = Rect::new(0.0, 0.0, SIDEBAR_WIDTH, 120.0);
    let rows = desc.rows();
    let footer_top = desc.rect.max().y - SIDEBAR_PADDING - desc.footer_height();
    for row in &rows {
        match row.kind {
            SidebarRowKind::FooterItem { .. } => assert!(row.rect.origin.y >= footer_top),
            _ => assert!(row.rect.max().y <= footer_top),
        }
    }
    assert!(rows.iter().any(|row| row.rect.max().y <= footer_top));
}
