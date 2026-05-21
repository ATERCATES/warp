use enum_iterator::all;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, SingletonEntity};

use crate::appearance::Appearance;
use crate::terminal::cli_agent::CLIAgent;
use crate::terminal::remote_sessions::RemoteTmuxSession;
use crate::ui_components::icon_with_status::{render_icon_with_status, IconWithStatusVariant};

use super::view::RemoteSessionsPanelAction;

const VERTICAL_TABS_ICON_SIZE: f32 = 24.;
const ICON_WITH_STATUS_GAP: f32 = 8.;
const ROW_CORNER_RADIUS: f32 = 4.;
const ROW_LEFT_INDENT: f32 = 24.;
const ACTION_ICON_BOX: f32 = 22.;
const ACTION_ICON_SIZE: f32 = 12.;
const ACTION_GAP: f32 = 2.;

pub struct SessionRowProps<'a> {
    pub host_key: String,
    pub session: &'a RemoteTmuxSession,
    pub mouse_state: MouseStateHandle,
    pub kill_button_state: MouseStateHandle,
}

pub fn render_session_row(props: SessionRowProps<'_>, app: &AppContext) -> Box<dyn Element> {
    let SessionRowProps {
        host_key,
        session,
        mouse_state,
        kill_button_state,
    } = props;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let session_name = session.name.clone();
    let attached_count = session.attached_count;
    let is_attached = attached_count > 0;
    let current_command = session.current_command.clone();
    let detected_agent = detect_cli_agent(&current_command);
    let host_key_click = host_key.clone();
    let session_name_click = session_name.clone();
    let host_key_kill = host_key.clone();
    let session_name_kill = session_name.clone();

    Hoverable::new(mouse_state, move |mouse_state| {
        let icon_variant = match detected_agent {
            Some(agent) => IconWithStatusVariant::CLIAgent {
                agent,
                status: None,
                is_ambient: false,
            },
            None => IconWithStatusVariant::Neutral {
                icon: Icon::Terminal,
                icon_color: theme.sub_text_color(theme.background()),
            },
        };
        let icon = render_icon_with_status(
            icon_variant,
            VERTICAL_TABS_ICON_SIZE,
            0.,
            theme,
            theme.background(),
        );

        let title = Text::new_inline(session_name.clone(), font_family, 12.)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
        let subtitle_text = format_subtitle(&current_command, is_attached, attached_count);
        let subtitle = Text::new_inline(subtitle_text, font_family, 12.)
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();

        let content_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(2.)
            .with_child(title)
            .with_child(subtitle)
            .finish();

        let mut actions = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ACTION_GAP);
        if mouse_state.is_hovered() {
            let kill_action = RemoteSessionsPanelAction::KillSession {
                key: host_key_kill.clone(),
                name: session_name_kill.clone(),
            };
            actions = actions.with_child(render_icon_action(
                Icon::Trash,
                kill_action,
                kill_button_state.clone(),
                appearance,
            ));
        }

        let leading = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(ICON_WITH_STATUS_GAP)
            .with_child(icon)
            .with_child(content_col)
            .finish();

        let content = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., leading).finish())
            .with_child(actions.finish())
            .finish();

        let mut container = Container::new(content)
            .with_padding_left(ROW_LEFT_INDENT)
            .with_padding_right(8.)
            .with_vertical_padding(8.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(ROW_CORNER_RADIUS)));
        if mouse_state.is_hovered() {
            container = container.with_background(internal_colors::fg_overlay_1(theme));
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(RemoteSessionsPanelAction::AttachSession {
            key: host_key_click.clone(),
            name: session_name_click.clone(),
        });
    })
    .finish()
}

fn detect_cli_agent(current_command: &str) -> Option<CLIAgent> {
    let cmd = current_command.trim();
    if cmd.is_empty() {
        return None;
    }
    let first_token = cmd.split_whitespace().next().unwrap_or("");
    let basename = first_token.rsplit('/').next().unwrap_or(first_token);
    all::<CLIAgent>().find(|a| {
        let prefix = a.command_prefix();
        !prefix.is_empty() && basename == prefix
    })
}

fn format_subtitle(current_command: &str, is_attached: bool, attached_count: u32) -> String {
    let trimmed = current_command.trim();
    let suffix = if is_attached {
        if attached_count == 1 {
            " · live".to_string()
        } else {
            format!(" · {attached_count}× live")
        }
    } else {
        " · idle".to_string()
    };
    if trimmed.is_empty() {
        format!("(no command){suffix}")
    } else {
        format!("{trimmed}{suffix}")
    }
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
