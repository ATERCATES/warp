use pathfinder_color::ColorU;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, SingletonEntity};

use crate::appearance::Appearance;
use crate::settings::remote_hosts::RemoteHost;
use crate::terminal::remote_sessions::{HostState, HostStatus};
use crate::ui_components::icon_with_status::{render_icon_with_status, IconWithStatusVariant};

use super::view::RemoteSessionsPanelAction;

const VERTICAL_TABS_ICON_SIZE: f32 = 24.;
const ICON_WITH_STATUS_GAP: f32 = 8.;
const ROW_CORNER_RADIUS: f32 = 4.;
const ROW_PADDING: f32 = 8.;
const STATUS_DOT_SIZE: f32 = 8.;
const STATUS_DOT_OFFSET: f32 = -2.;
const ACTION_ICON_BOX: f32 = 22.;
const ACTION_ICON_SIZE: f32 = 12.;
const ACTION_GAP: f32 = 2.;
const DEFAULT_PORT: u16 = 22;

pub struct HostRowProps<'a> {
    pub state: &'a HostState,
    pub host: &'a RemoteHost,
    pub expanded: bool,
    pub mouse_state: MouseStateHandle,
    pub connect_button_state: MouseStateHandle,
    pub disconnect_button_state: MouseStateHandle,
    pub remove_button_state: MouseStateHandle,
    pub confirm_remove_state: MouseStateHandle,
    pub cancel_remove_state: MouseStateHandle,
    pub is_pending_remove: bool,
}

pub fn render_host_row(props: HostRowProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let HostRowProps {
        state,
        host,
        expanded,
        mouse_state,
        connect_button_state,
        disconnect_button_state,
        remove_button_state,
        confirm_remove_state,
        cancel_remove_state,
        is_pending_remove,
    } = props;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let key = state.local_host_key.clone();
    let alias = host.alias.clone();
    let host_target = if host.port == DEFAULT_PORT {
        host.host.clone()
    } else {
        format!("{}:{}", host.host, host.port)
    };
    let status = state.status.clone();
    let subtitle = subtitle_summary(state, &host_target);
    let error_detail = state
        .last_error_detail
        .clone()
        .filter(|_| matches!(state.status, HostStatus::Error(_)));
    let font_family = appearance.ui_font_family();
    let is_connected = matches!(status, HostStatus::Connected);
    let is_disconnected = matches!(status, HostStatus::Disconnected);
    let key_for_click = key.clone();

    Hoverable::new(mouse_state, move |mouse_state| {
        let base_icon = render_icon_with_status(
            IconWithStatusVariant::Neutral {
                icon: Icon::RemoteServer,
                icon_color: theme.main_text_color(theme.background()),
            },
            VERTICAL_TABS_ICON_SIZE,
            0.,
            theme,
            theme.background(),
        );
        let icon = with_status_dot(base_icon, status_dot_color(&status, theme));

        let title = Text::new_inline(alias.clone(), font_family, 12.)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
        let subtitle_text = Text::new_inline(subtitle.clone(), font_family, 12.)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();

        let content_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(title)
            .with_child(subtitle_text)
            .finish();

        let trailing = if is_pending_remove {
            render_confirm_remove(
                confirm_remove_state.clone(),
                cancel_remove_state.clone(),
                appearance,
            )
        } else {
            render_actions_row(
                &status,
                expanded,
                &key,
                connect_button_state.clone(),
                disconnect_button_state.clone(),
                remove_button_state.clone(),
                mouse_state.is_hovered(),
                appearance,
            )
        };

        let row_content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ICON_WITH_STATUS_GAP)
            .with_child(icon)
            .with_child(Shrinkable::new(1.0, content_col).finish())
            .with_child(trailing)
            .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(row_content);
        if let Some(detail) = error_detail.clone() {
            column.add_child(
                Container::new(
                    Text::new_inline(detail, font_family, 12.)
                        .with_color(theme.ui_error_color().into())
                        .finish(),
                )
                .with_padding_left(VERTICAL_TABS_ICON_SIZE + ICON_WITH_STATUS_GAP)
                .with_margin_top(3.)
                .finish(),
            );
        }

        let mut container = Container::new(column.finish())
            .with_uniform_padding(ROW_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)));
        if mouse_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        if is_connected {
            ctx.dispatch_typed_action(RemoteSessionsPanelAction::ToggleHostExpanded {
                key: key_for_click.clone(),
            });
        } else if is_disconnected {
            ctx.dispatch_typed_action(RemoteSessionsPanelAction::ConnectHost {
                key: key_for_click.clone(),
            });
        }
    })
    .finish()
}

fn with_status_dot(icon: Box<dyn Element>, dot_color: ColorU) -> Box<dyn Element> {
    let dot = ConstrainedBox::new(
        Container::new(Flex::row().with_main_axis_size(MainAxisSize::Min).finish())
            .with_background(dot_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish(),
    )
    .with_width(STATUS_DOT_SIZE)
    .with_height(STATUS_DOT_SIZE)
    .finish();
    let mut stack = Stack::new();
    stack.add_child(icon);
    stack.add_positioned_overlay_child(
        dot,
        OffsetPositioning::offset_from_parent(
            pathfinder_geometry::vector::vec2f(STATUS_DOT_OFFSET, STATUS_DOT_OFFSET),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            warpui::elements::ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(VERTICAL_TABS_ICON_SIZE)
        .with_height(VERTICAL_TABS_ICON_SIZE)
        .finish()
}

fn subtitle_summary(state: &HostState, host_target: &str) -> String {
    match &state.status {
        HostStatus::Disconnected => host_target.to_string(),
        HostStatus::Connecting => format!("{host_target} · connecting…"),
        HostStatus::Connected => {
            let total = state.sessions.len();
            let attached: u32 = state
                .sessions
                .iter()
                .map(|s| if s.attached_count > 0 { 1 } else { 0 })
                .sum();
            let caps = state
                .capabilities
                .as_ref()
                .map(|c| format!(" · tmux {}", c.tmux_version))
                .unwrap_or_default();
            format!("{host_target} · {attached}/{total} attached{caps}")
        }
        HostStatus::Error(_) => format!("{host_target} · error"),
        HostStatus::Unsupported(msg) => format!("{host_target} · unsupported: {msg}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_actions_row(
    status: &HostStatus,
    expanded: bool,
    key: &str,
    connect_button_state: MouseStateHandle,
    disconnect_button_state: MouseStateHandle,
    remove_button_state: MouseStateHandle,
    is_row_hovered: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(ACTION_GAP);

    match status {
        HostStatus::Connected => {
            if is_row_hovered {
                row = row.with_child(render_icon_action(
                    Icon::LogOut,
                    RemoteSessionsPanelAction::DisconnectHost {
                        key: key.to_string(),
                    },
                    disconnect_button_state,
                    appearance,
                ));
                row = row.with_child(render_icon_action(
                    Icon::Trash,
                    RemoteSessionsPanelAction::RequestRemoveHost {
                        key: key.to_string(),
                    },
                    remove_button_state,
                    appearance,
                ));
            }
            let chevron_icon = if expanded {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            };
            row = row.with_child(
                ConstrainedBox::new(
                    chevron_icon
                        .to_warpui_icon(theme.sub_text_color(theme.background()))
                        .finish(),
                )
                .with_width(ACTION_ICON_SIZE)
                .with_height(ACTION_ICON_SIZE)
                .finish(),
            );
        }
        HostStatus::Disconnected => {
            if is_row_hovered {
                row = row.with_child(render_icon_action(
                    Icon::Trash,
                    RemoteSessionsPanelAction::RequestRemoveHost {
                        key: key.to_string(),
                    },
                    remove_button_state,
                    appearance,
                ));
            }
            row = row.with_child(
                Container::new(
                    Text::new_inline("Connect", appearance.ui_font_family(), 11.)
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .with_horizontal_padding(4.)
                .finish(),
            );
            let _ = connect_button_state;
        }
        HostStatus::Connecting => {
            row = row.with_child(
                Container::new(
                    Text::new_inline("…", appearance.ui_font_family(), 12.)
                        .with_color(theme.sub_text_color(theme.background()).into())
                        .finish(),
                )
                .with_horizontal_padding(4.)
                .finish(),
            );
            let _ = connect_button_state;
            let _ = disconnect_button_state;
        }
        HostStatus::Error(_) => {
            row = row.with_child(render_icon_action(
                Icon::Refresh,
                RemoteSessionsPanelAction::ConnectHost {
                    key: key.to_string(),
                },
                connect_button_state,
                appearance,
            ));
            if is_row_hovered {
                row = row.with_child(render_icon_action(
                    Icon::Trash,
                    RemoteSessionsPanelAction::RequestRemoveHost {
                        key: key.to_string(),
                    },
                    remove_button_state,
                    appearance,
                ));
            }
        }
        HostStatus::Unsupported(_) => {
            if is_row_hovered {
                row = row.with_child(render_icon_action(
                    Icon::Trash,
                    RemoteSessionsPanelAction::RequestRemoveHost {
                        key: key.to_string(),
                    },
                    remove_button_state,
                    appearance,
                ));
            }
            let _ = connect_button_state;
        }
    }
    let _ = expanded;
    row.finish()
}

fn render_confirm_remove(
    confirm_state: MouseStateHandle,
    cancel_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let prompt = Text::new_inline("Remove?", appearance.ui_font_family(), 11.)
        .with_color(theme.ui_error_color().into())
        .finish();
    let cancel = render_icon_action(
        Icon::Cancelled,
        RemoteSessionsPanelAction::CancelRemoveHost,
        cancel_state,
        appearance,
    );
    let confirm = render_icon_action(
        Icon::Check,
        RemoteSessionsPanelAction::ConfirmRemoveHost,
        confirm_state,
        appearance,
    );
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(ACTION_GAP)
        .with_child(prompt)
        .with_child(cancel)
        .with_child(confirm)
        .finish()
}

fn render_icon_action(
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
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
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

fn status_dot_color(status: &HostStatus, theme: &warp_core::ui::theme::WarpTheme) -> ColorU {
    match status {
        HostStatus::Disconnected => theme.sub_text_color(theme.background()).into_solid(),
        HostStatus::Connecting => theme.ansi_fg_yellow(),
        HostStatus::Connected => theme.ansi_fg_green(),
        HostStatus::Error(_) | HostStatus::Unsupported(_) => theme.ui_error_color(),
    }
}
