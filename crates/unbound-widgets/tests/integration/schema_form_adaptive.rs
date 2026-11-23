//! `SchemaForm` 自适应 CJK 标签列、超长标签省略与行尾动作槽集成测试。
//! lang-allow: CJK 标签宽度自适应与省略行为必须以中文字面量作为测量输入

use unbound_input::pick;
use unbound_math::Rect;
use unbound_widgets::elided_text::ELLIPSIS;
use unbound_widgets::schema_form::{
    emit_schema_form, estimate_label_text_width, schema_form_trailing_action_node,
    FieldInheritanceState, SchemaFieldKind, SchemaFieldSpec, SchemaFormDesc, SchemaFormEvent,
    SchemaFormState, SchemaValue, DEFAULT_LABEL_MAX_RATIO, DEFAULT_LABEL_MAX_WIDTH,
    DEFAULT_LABEL_MIN_WIDTH, DEFAULT_TRAILING_ACTION_WIDTH, LABEL_CONTROL_GAP, LABEL_FONT_SIZE,
    TRAILING_ACTION_GAP,
};
use unbound_widgets::{ControlState, PointerPhase, SceneCollector};

fn sample_cjk_fields() -> Vec<SchemaFieldSpec> {
    vec![
        SchemaFieldSpec::new(
            "shader_cache_dir",
            "着色器编译缓存输出目录路径",
            "构建",
            SchemaFieldKind::Text {
                placeholder: "请输入目录路径",
                required: true,
            },
            SchemaValue::Text(String::from("D:/cache/shaders")),
        )
        .with_inheritance(FieldInheritanceState::InheritedFrom("DefaultPreset".into()))
        .with_trailing_action("重置"),
        SchemaFieldSpec::new(
            "enable_validation",
            "启用图形管线校验层与实时诊断捕获",
            "诊断",
            SchemaFieldKind::Switch,
            SchemaValue::Bool(true),
        )
        .with_inheritance(FieldInheritanceState::Overridden)
        .with_trailing_action("还原"),
        SchemaFieldSpec::new(
            "frame_budget_ms",
            "单帧渲染预算上限",
            "性能",
            SchemaFieldKind::Number {
                min: 1.0,
                max: 60.0,
                step: 0.5,
                unit: "ms",
            },
            SchemaValue::Number(16.6),
        ),
    ]
}

#[test]
fn cjk_long_labels_never_overlap_controls_across_800_1280_1920() {
    let fields = sample_cjk_fields();

    for width in [800.0_f32, 1280.0_f32, 1920.0_f32] {
        let rect = Rect::new(24.0, 32.0, width, 240.0);
        let desc = SchemaFormDesc::new(rect, fields.clone());

        let col_w = desc.resolve_label_column_width(width);
        let expected_upper = (width * DEFAULT_LABEL_MAX_RATIO).min(DEFAULT_LABEL_MAX_WIDTH);
        assert!(
            col_w >= DEFAULT_LABEL_MIN_WIDTH && col_w <= expected_upper,
            "width={width}: adaptive label column width {col_w} out of [{DEFAULT_LABEL_MIN_WIDTH}, {expected_upper}]"
        );

        let rows = desc.layout_rows();
        assert_eq!(rows.len(), fields.len());
        for row in &rows {
            // 标签矩形右边界严格小于输入控件左边界，且保留至少 12px 间距。
            assert!(
                !row.label_rect.intersects(&row.control_rect),
                "width={width}, row={}: label_rect {:?} overlaps control_rect {:?}",
                row.index,
                row.label_rect,
                row.control_rect
            );
            assert!(
                row.control_rect.origin.x - row.label_rect.max().x >= LABEL_CONTROL_GAP - 1e-3,
                "width={width}, row={}: gap {} < {LABEL_CONTROL_GAP}",
                row.index,
                row.control_rect.origin.x - row.label_rect.max().x
            );
            // 实际显示文本宽度严格不超过标签可用列宽。
            let display_w = estimate_label_text_width(&row.display_label, LABEL_FONT_SIZE);
            assert!(
                display_w <= row.label_rect.size.x + 1e-3,
                "width={width}, row={}: display_label {:?} width {display_w} exceeds label_rect width {}",
                row.index,
                row.display_label,
                row.label_rect.size.x
            );
        }
    }
}

#[test]
fn overly_long_cjk_label_automatically_elides_and_respects_builder_overrides() {
    let super_long_label =
        "超长中文配置项标签用于验证超出自适应上限时按省略规则自动截断绝不与右侧输入控件发生重叠";
    let fields = vec![SchemaFieldSpec::new(
        "long_key",
        super_long_label,
        "常规",
        SchemaFieldKind::Text {
            placeholder: "",
            required: false,
        },
        SchemaValue::Text(String::from("value")),
    )
    .with_inheritance(FieldInheritanceState::InheritedFrom("BaseConfig".into()))];

    let rect = Rect::new(0.0, 0.0, 800.0, 120.0);
    let desc = SchemaFormDesc::new(rect, fields.clone());
    let rows = desc.layout_rows();
    let row = &rows[0];

    assert!(row.label_elided, "overly long CJK label must be elided");
    assert!(
        row.display_label.ends_with(ELLIPSIS),
        "elided label should end with {ELLIPSIS:?}, got {:?}",
        row.display_label
    );
    assert!(
        estimate_label_text_width(&row.display_label, LABEL_FONT_SIZE)
            <= row.label_rect.size.x + 1e-3
    );
    assert!(row.control_rect.origin.x - row.label_rect.max().x >= LABEL_CONTROL_GAP - 1e-3);

    // 验证显式 builder 覆写最小/最大宽与比例。
    let custom = SchemaFormDesc::new(rect, fields)
        .with_label_width_bounds(180.0, 240.0)
        .with_label_max_ratio(0.30);
    let custom_w = custom.resolve_label_column_width(800.0);
    assert!(
        (custom_w - 240.0).abs() < 1e-3,
        "expected custom clamped width 240.0, got {custom_w}"
    );
}

#[test]
fn trailing_action_slot_shrinks_control_without_overlap_and_is_clickable() {
    let fields = sample_cjk_fields();
    let rect = Rect::new(16.0, 20.0, 1280.0, 220.0);
    let desc = SchemaFormDesc::new(rect, fields.clone());
    let state = SchemaFormState::from_schema(&fields);
    let rows = desc.layout_rows();

    // 第 0、1 行有行尾动作，第 2 行无行尾动作。
    let action_r0 = rows[0]
        .trailing_action_rect
        .expect("row 0 should have trailing action rect");
    let action_r1 = rows[1]
        .trailing_action_rect
        .expect("row 1 should have trailing action rect");
    assert!(
        rows[2].trailing_action_rect.is_none(),
        "row 2 without trailing_action_label must not reserve trailing slot"
    );

    // 有行尾动作时输入控件自动收缩，与动作按钮零重叠并保留 TRAILING_ACTION_GAP。
    assert!(
        rows[0].control_rect.size.x < rows[2].control_rect.size.x,
        "row with trailing action should shrink control width"
    );
    assert!((action_r0.size.x - DEFAULT_TRAILING_ACTION_WIDTH).abs() < 1e-3);
    assert!(!rows[0].control_rect.intersects(&action_r0));
    assert!(action_r0.origin.x - rows[0].control_rect.max().x >= TRAILING_ACTION_GAP - 1e-3);
    assert!(!rows[1].control_rect.intersects(&action_r1));
    assert!(action_r1.origin.x - rows[1].control_rect.max().x >= TRAILING_ACTION_GAP - 1e-3);

    // 几何 hit_test 精确返回 TrailingAction(idx) 与 ControlClicked(idx)。
    assert_eq!(
        desc.hit_test(action_r0.center()),
        Some(SchemaFormEvent::TrailingAction(0))
    );
    assert_eq!(
        desc.hit_test(action_r1.center()),
        Some(SchemaFormEvent::TrailingAction(1))
    );
    assert_eq!(
        desc.hit_test(rows[0].control_rect.center()),
        Some(SchemaFormEvent::ControlClicked(0))
    );

    // emit_schema_form 注册的子命中区域可被输入路由器精确拾取。
    let base_node = 4096_u64;
    let mut scene = SceneCollector::new(1);
    let out = emit_schema_form(base_node, &desc, &state, ControlState::default(), &[]);
    scene.push(base_node, out.clone());

    let picked0 =
        pick(&scene.regions, action_r0.center()).expect("trailing action 0 must be pickable");
    assert_eq!(picked0.node, schema_form_trailing_action_node(base_node, 0));

    let picked1 =
        pick(&scene.regions, action_r1.center()).expect("trailing action 1 must be pickable");
    assert_eq!(picked1.node, schema_form_trailing_action_node(base_node, 1));

    // 24 态恒定图元数契约验证（含行尾动作槽）。
    let base_prim_count = out.primitives.len();
    for phase in [
        PointerPhase::Idle,
        PointerPhase::Hovered,
        PointerPhase::Pressed,
    ] {
        for focused in [false, true] {
            for disabled in [false, true] {
                for selected in [false, true] {
                    let st = ControlState {
                        phase,
                        focused,
                        disabled,
                        selected,
                    };
                    let probed = emit_schema_form(base_node, &desc, &state, st, &[]);
                    assert_eq!(probed.primitives.len(), base_prim_count);
                }
            }
        }
    }
}
