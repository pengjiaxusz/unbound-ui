//! 后景模式对照页（Backdrop）。
//!
//! 支持在运行时切换 `Auto` / `Native` / `SelfDrawn` / `Off` 四条路径，并回读宿主
//! 解析后的实际生效模式。页面同时显示平台探测得到的 Windows 构建号与原生可用性，
//! 并在原生不可用时明确提示 `Native` / `Auto` 会回退到自绘。

use accesskit::Role;
use unbound_backdrop::BackdropMode;
use unbound_math::Rect;

use super::{DemoCtx, DemoKind, ExampleDef, PageInfo};
use crate::canvas::{AlignX, AlignY};

pub const BACKDROP_INFO: PageInfo = PageInfo {
    title: "Backdrop Modes & Native Mica",
    short: "Backdrop",
    description: "Compare the DWM-native Mica path, the self-drawn wallpaper blur and the disabled backdrop; switch modes at runtime and read back the effective path resolved by the host.",
    crates: &[
        "unbound-backdrop",
        "unbound-app",
        "unbound-platform",
        "unbound-gpu",
    ],
    keywords: "backdrop mica dwm native self-drawn wallpaper blur transparency swapchain auto off windows build",
};

pub const BACKDROP_EXAMPLES: &[ExampleDef] = &[ExampleDef::new(
    "Backdrop Mode Switcher",
    "Pick Auto, Native, SelfDrawn or Off. The host resolves the choice on the next frame and reports the effective mode without restarting the application.",
    &["unbound-backdrop", "unbound-app"],
    r#"let resolved = ResolvedBackdrop::resolve(state.choice, native_available);
if resolved != state.effective {
    state.effective = resolved;
    request_rebuild();
}"#,
    "Four mode buttons publish the Button role, an accessible name and the selected state; the readout shows the user choice, effective mode, Windows build and native availability.",
    &[],
    DemoKind::Backdrop,
)
.with_height(340.0)];

/// 渲染后景模式对照舞台。
pub fn backdrop(ctx: &mut DemoCtx<'_, '_>) {
    let rect = ctx.rect;
    let palette = *ctx.canvas.palette;
    let choice = ctx.state.backdrop_mode;
    let resolved = ctx.state.resolved_backdrop;
    let native_available = ctx.state.native_backdrop_available;
    let windows_build = ctx.state.windows_build;

    let info = Rect::new(
        rect.origin.x + 6.0,
        rect.origin.y + 6.0,
        rect.size.x - 12.0,
        88.0,
    );
    ctx.canvas
        .rect(info, 10.0, palette.card_bg, palette.card_border, 1.0, 0.0);

    let build_text = windows_build
        .map(|build| format!("Build {build}"))
        .unwrap_or_else(|| "Unknown".to_string());
    let availability_text = if native_available {
        "Available".to_string()
    } else {
        "Unavailable".to_string()
    };
    let rows = [
        (
            "USER CHOICE",
            choice.label().to_string(),
            choice == BackdropMode::Auto,
        ),
        ("EFFECTIVE MODE", resolved.mode().label().to_string(), false),
        ("WINDOWS BUILD", build_text, false),
        ("NATIVE AVAILABLE", availability_text, false),
    ];
    let column_gap = 14.0;
    let column_w = ((info.size.x - 24.0 - column_gap) / 2.0).max(120.0);
    let row_h = 38.0;
    for (index, (label, value, highlight)) in rows.iter().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let cell = Rect::new(
            info.origin.x + 12.0 + column as f32 * (column_w + column_gap),
            info.origin.y + 6.0 + row as f32 * row_h,
            column_w,
            row_h - 4.0,
        );
        ctx.canvas.text(
            label,
            Rect::new(cell.origin.x, cell.origin.y, cell.size.x, 12.0),
            8.5,
            palette.text_muted,
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text(
            value,
            Rect::new(
                cell.origin.x,
                cell.origin.y + 13.0,
                cell.size.x,
                cell.size.y - 13.0,
            ),
            12.0,
            if *highlight {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
    }

    let hint_h = 26.0;
    let grid_top = info.max().y + 8.0;
    let grid_h = (rect.max().y - grid_top - hint_h - 6.0).max(96.0);
    let gap = 8.0;
    let cell_w = ((rect.size.x - gap * 3.0) / 4.0).max(72.0);
    for mode in BackdropMode::ALL {
        let part = 1 + mode.index() as u16;
        let cell = Rect::new(
            rect.origin.x + mode.index() as f32 * (cell_w + gap),
            grid_top,
            cell_w,
            grid_h,
        );
        let selected = mode == choice;
        let hover = ctx.canvas.visual(ctx.part(part), selected);
        let background = if selected {
            palette.primary_soft
        } else if hover.is_hovered() {
            palette.control_hover
        } else {
            palette.card_bg
        };
        let border = if selected {
            palette.primary
        } else {
            palette.card_border
        };
        ctx.canvas.rect(
            cell,
            8.0,
            background,
            border,
            if selected { 1.5 } else { 1.0 },
            0.0,
        );
        ctx.canvas.text(
            mode.label(),
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + 8.0,
                (cell.size.x - 20.0).max(20.0),
                18.0,
            ),
            11.5,
            if selected {
                palette.primary
            } else {
                palette.text
            },
            AlignX::Start,
            AlignY::Center,
        );
        ctx.canvas.text_wrapped(
            mode.description(),
            Rect::new(
                cell.origin.x + 10.0,
                cell.origin.y + 30.0,
                (cell.size.x - 20.0).max(20.0),
                (cell.size.y - 58.0).max(24.0),
            ),
            9.0,
            palette.text_muted,
        );
        if selected {
            ctx.canvas.text(
                "SELECTED",
                Rect::new(
                    cell.origin.x + 10.0,
                    cell.max().y - 26.0,
                    (cell.size.x - 20.0).max(20.0),
                    16.0,
                ),
                8.5,
                palette.primary,
                AlignX::Start,
                AlignY::Center,
            );
        }
        ctx.control(
            part,
            cell,
            Role::Button,
            &format!("{} backdrop mode — {}", mode.label(), mode.description()),
            selected,
        );
    }

    let hint = if !native_available && matches!(choice, BackdropMode::Auto | BackdropMode::Native) {
        "Native path is unavailable (requires Windows 11 build 22621+ with premultiplied alpha); Auto and Native fall back to the self-drawn texture."
    } else if resolved.uses_native_path() {
        "Native path active: the DWM Mica material composites behind a transparent swapchain."
    } else if resolved.draws_self_drawn_texture() {
        "Self-drawn path selected: an approximation built from the primary wallpaper (cover fit, blurred, sampled at this window's screen rect); it cannot capture content behind the window."
    } else {
        "Backdrop disabled: the root surface uses the opaque theme colour."
    };
    ctx.canvas.text(
        hint,
        Rect::new(
            rect.origin.x + 10.0,
            rect.max().y - hint_h + 4.0,
            (rect.size.x - 20.0).max(40.0),
            hint_h - 6.0,
        ),
        9.0,
        palette.text_muted,
        AlignX::Start,
        AlignY::Center,
    );
}
