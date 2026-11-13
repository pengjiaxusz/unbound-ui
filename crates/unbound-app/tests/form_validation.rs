//! Integration tests for FormController & FormValidator (S5.3).

use unbound_math::Rect;
use unbound_widgets::form::{
    layout_form, FormController, FormDesc, FormField, FormFieldDesc, FormValidator,
};
use unbound_widgets::scroll::ScrollState;

const F_NAME: u64 = 1;
const F_AGE: u64 = 2;
const F_EMAIL: u64 = 3;

#[test]
fn form_controller_tracks_dirty_touched_and_scrolls_to_first_error_on_submit() {
    let mut ctrl = FormController::new();
    ctrl.register_field(
        F_NAME,
        "Alice",
        FormValidator::new()
            .required("Name required")
            .min_len(2, "Too short"),
    );
    ctrl.register_field(
        F_AGE,
        "25",
        FormValidator::new().range(Some(18.0), Some(120.0), "Must be 18..120"),
    );
    ctrl.register_field(
        F_EMAIL,
        "",
        FormValidator::new()
            .required("Email required")
            .pattern("@", "Invalid email")
            .custom(|s| {
                (!s.ends_with(".invalid"))
                    .then_some(())
                    .ok_or_else(|| "Blocked domain".to_string())
                    .err()
            }),
    );

    assert!(!ctrl.any_dirty());
    assert!(!ctrl.is_touched(F_EMAIL));

    // Modify F_NAME -> dirty, not touched yet so no error until touched
    ctrl.set_value(F_NAME, "A");
    assert!(ctrl.is_dirty(F_NAME));
    assert!(ctrl.state().error(F_NAME).is_none());

    ctrl.mark_touched(F_NAME);
    assert!(ctrl.is_touched(F_NAME));
    assert_eq!(ctrl.state().error(F_NAME), Some("Too short"));

    // Fix F_NAME, break F_AGE and F_EMAIL
    ctrl.set_value(F_NAME, "Bob");
    assert!(ctrl.state().error(F_NAME).is_none());
    ctrl.set_value(F_AGE, "12");
    ctrl.set_value(F_EMAIL, "bad");

    let fields = vec![
        FormField::new(F_NAME, FormFieldDesc::new("Name").required(true)),
        FormField::new(F_AGE, FormFieldDesc::new("Age")),
        FormField::new(F_EMAIL, FormFieldDesc::new("Email").required(true)),
    ];
    let viewport = Rect::new(0.0, 0.0, 320.0, 90.0);
    let layouts = layout_form(&FormDesc::new(Rect::new(0.0, 0.0, 320.0, 400.0)), &fields);
    let mut scroll = ScrollState::default();

    let res = ctrl.submit(&layouts, Some((&mut scroll, viewport, 400.0)));
    assert_eq!(res, Err(F_AGE));
    assert_eq!(ctrl.state().focused(), Some(F_AGE));
    assert!(scroll.target().y > 0.0, "scroll must reveal failing F_AGE");

    // Fix remaining fields and resubmit
    ctrl.set_value(F_AGE, "30");
    ctrl.set_value(F_EMAIL, "bob@example.com");
    assert_eq!(ctrl.submit(&layouts, None), Ok(()));
    assert!(ctrl.state().is_valid());
}
