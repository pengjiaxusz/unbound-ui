//! Integration tests for RouterStack & breadcrumb synchronization (S5.4).

use unbound_app::router::{RouteEntry, RouterStack, TransitionDirection};

#[test]
fn router_push_pop_forward_replace_and_breadcrumb_sync() {
    let mut router = RouterStack::new(RouteEntry::new(1, "home", "Home"));
    assert_eq!(router.current().name, "home");
    assert!(!router.can_pop());
    assert!(!router.can_forward());

    router.push(RouteEntry::new(2, "projects", "Projects").with_param("org", "unbound"));
    router.push(RouteEntry::new(3, "detail", "Editor").with_param("id", "42"));
    assert_eq!(router.current().param("id"), Some("42"));
    assert_eq!(router.transition().0, TransitionDirection::Forward);

    let crumbs = router.breadcrumb_items();
    assert_eq!(crumbs.len(), 3);
    assert_eq!(crumbs[0].label, "Home");
    assert_eq!(crumbs[1].label, "Projects");
    assert_eq!(crumbs[2].label, "Editor");

    // Advance transition to completion
    assert!(router.tick(0.5));
    assert_eq!(router.transition(), (TransitionDirection::None, 1.0));

    // Pop and go forward
    assert!(router.pop());
    assert_eq!(router.current().name, "projects");
    assert_eq!(router.current().param("org"), Some("unbound"));
    assert!(router.can_forward());

    assert!(router.forward());
    assert_eq!(router.current().name, "detail");

    // Replace top
    router.replace(RouteEntry::new(4, "settings", "Settings"));
    assert_eq!(router.history().len(), 3);
    assert_eq!(router.current().name, "settings");

    // Jump via breadcrumb index 0
    assert!(router.pop_to_index(0));
    assert_eq!(router.current().name, "home");
    assert!(!router.can_pop());
}
