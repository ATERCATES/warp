use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, Element, Hoverable, MouseStateHandle, Radius,
};
use warpui::platform::Cursor;

use crate::appearance::Appearance;

use super::view::RemoteSessionsPanelAction;

pub(super) const VERTICAL_TABS_ICON_SIZE: f32 = 24.;
pub(super) const ICON_WITH_STATUS_GAP: f32 = 8.;
pub(super) const ROW_CORNER_RADIUS: f32 = 4.;
pub(super) const ACTION_ICON_SIZE: f32 = 12.;
pub(super) const ACTION_GAP: f32 = 2.;
const ACTION_ICON_BOX: f32 = 22.;
const ACTION_BUTTON_CORNER_RADIUS: f32 = 4.;

pub(super) fn render_icon_action(
    icon: Icon,
    action: RemoteSessionsPanelAction,
    button_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let pad = (ACTION_ICON_BOX - ACTION_ICON_SIZE) / 2.;
    Hoverable::new(button_state, move |mouse_state| {
        let color: Fill = if mouse_state.is_hovered() {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        let glyph = ConstrainedBox::new(icon.to_warpui_icon(color).finish())
            .with_width(ACTION_ICON_SIZE)
            .with_height(ACTION_ICON_SIZE)
            .finish();
        let mut container = Container::new(glyph)
            .with_uniform_padding(pad)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ACTION_BUTTON_CORNER_RADIUS)));
        if mouse_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_2(theme));
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(action.clone());
    })
    .finish()
}
