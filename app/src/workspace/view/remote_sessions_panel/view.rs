use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use warp_core::ui::Icon;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Fill, Flex,
    FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth,
    Shrinkable, Text, UniformList, UniformListState,
};
use warpui::platform::Cursor;
use warpui::text_layout::TextAlignment;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::settings::remote_hosts::{
    RemoteHost, RemoteSessionsSettings, RemoteSessionsSettingsChangedEvent,
};
use crate::terminal::remote_sessions::{
    HostStatus, RemoteSessionsEvent, RemoteSessionsModel,
};

use super::host_row::{render_host_row, HostRowProps};
use super::session_row::{render_session_row, SessionRowProps};

const PANEL_WIDTH: f32 = 320.;
const PANEL_HEADER_HORIZONTAL_PADDING: f32 = 12.;
const PANEL_HEADER_VERTICAL_PADDING: f32 = 10.;
const FOOTER_HORIZONTAL_PADDING: f32 = 12.;
const FOOTER_VERTICAL_PADDING: f32 = 10.;
const REFRESH_ICON_SIZE: f32 = 16.;
const EMPTY_STATE_ICON_SIZE: f32 = 32.;
const EMPTY_STATE_SPACING: f32 = 10.;

#[derive(Clone, Debug)]
pub enum RemoteSessionsPanelAction {
    Refresh,
    ConnectHost { key: String },
    DisconnectHost { key: String },
    ToggleHostExpanded { key: String },
    CreateDefaultSession { key: String },
    AttachSession { key: String, name: String },
    KillSession { key: String, name: String },
    OpenAddHostSettings,
    RequestRemoveHost { key: String },
    ConfirmRemoveHost,
    CancelRemoveHost,
}

#[derive(Clone, Debug)]
enum ListItem {
    Host(String),
    Session { host_key: String, session_index: usize },
    NewSessionRow { host_key: String },
}

#[derive(Default)]
struct StateHandles {
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    refresh_button: MouseStateHandle,
    add_host_button: MouseStateHandle,
    empty_state_button: MouseStateHandle,
    confirm_remove_button: MouseStateHandle,
    cancel_remove_button: MouseStateHandle,
    host_rows: std::collections::HashMap<String, MouseStateHandle>,
    connect_buttons: std::collections::HashMap<String, MouseStateHandle>,
    disconnect_buttons: std::collections::HashMap<String, MouseStateHandle>,
    remove_buttons: std::collections::HashMap<String, MouseStateHandle>,
    session_rows: std::collections::HashMap<String, MouseStateHandle>,
    kill_buttons: std::collections::HashMap<String, MouseStateHandle>,
    new_session_buttons: std::collections::HashMap<String, MouseStateHandle>,
}

impl StateHandles {
    fn new() -> Self {
        Self {
            list_state: UniformListState::new(),
            scroll_state: Arc::new(Mutex::new(Default::default())),
            ..Default::default()
        }
    }
}

pub struct RemoteSessionsPanelView {
    model: ModelHandle<RemoteSessionsModel>,
    expanded_hosts: HashSet<String>,
    list_items: Arc<Vec<ListItem>>,
    state_handles: StateHandles,
    pending_remove: Option<String>,
}

impl Entity for RemoteSessionsPanelView {
    type Event = ();
}

impl RemoteSessionsPanelView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model = RemoteSessionsModel::handle(ctx);
        let settings_handle = RemoteSessionsSettings::handle(ctx);

        let mut view = Self {
            model,
            expanded_hosts: HashSet::new(),
            list_items: Arc::new(Vec::new()),
            state_handles: StateHandles::new(),
            pending_remove: None,
        };

        view.ingest_settings_into_model(ctx);
        view.rebuild_list_items(ctx);

        ctx.subscribe_to_model(&view.model, |me, _, event, ctx| {
            me.handle_model_event(event, ctx);
        });
        ctx.subscribe_to_model(&settings_handle, |me, _, event, ctx| {
            me.handle_settings_event(event, ctx);
        });

        view
    }

    fn handle_model_event(
        &mut self,
        _event: &RemoteSessionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        self.rebuild_list_items(ctx);
        ctx.notify();
    }

    fn handle_settings_event(
        &mut self,
        event: &RemoteSessionsSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let RemoteSessionsSettingsChangedEvent::RemoteSessionsHosts { .. } = event {
            self.ingest_settings_into_model(ctx);
            self.rebuild_list_items(ctx);
            ctx.notify();
        }
    }

    fn ingest_settings_into_model(&mut self, ctx: &mut ViewContext<Self>) {
        let hosts: Vec<RemoteHost> = settings_hosts_snapshot(ctx);
        self.model.update(ctx, |model, ctx| {
            model.ingest_hosts(&hosts, ctx);
        });
    }

    fn rebuild_list_items(&mut self, ctx: &mut ViewContext<Self>) {
        let hosts = settings_hosts_snapshot(ctx);
        let model = self.model.as_ref(ctx);

        let live_host_keys: HashSet<String> =
            hosts.iter().map(|h| h.local_host_key.clone()).collect();
        self.state_handles
            .host_rows
            .retain(|k, _| live_host_keys.contains(k));
        self.state_handles
            .connect_buttons
            .retain(|k, _| live_host_keys.contains(k));
        self.state_handles
            .disconnect_buttons
            .retain(|k, _| live_host_keys.contains(k));
        self.state_handles
            .remove_buttons
            .retain(|k, _| live_host_keys.contains(k));
        self.state_handles
            .new_session_buttons
            .retain(|k, _| live_host_keys.contains(k));
        if let Some(key) = self.pending_remove.as_ref() {
            if !live_host_keys.contains(key) {
                self.pending_remove = None;
            }
        }
        let mut live_session_keys: HashSet<String> = HashSet::new();

        let mut items = Vec::new();
        for host in &hosts {
            let key = host.local_host_key.clone();
            self.state_handles
                .host_rows
                .entry(key.clone())
                .or_default();
            self.state_handles
                .connect_buttons
                .entry(key.clone())
                .or_default();
            self.state_handles
                .disconnect_buttons
                .entry(key.clone())
                .or_default();
            self.state_handles
                .remove_buttons
                .entry(key.clone())
                .or_default();
            items.push(ListItem::Host(key.clone()));
            if self.expanded_hosts.contains(&key) {
                if let Some(state) = model.host_state(&key) {
                    if matches!(state.status, HostStatus::Connected) {
                        for (idx, session) in state.sessions.iter().enumerate() {
                            let session_key = format!("{key}::{}", session.name);
                            self.state_handles
                                .session_rows
                                .entry(session_key.clone())
                                .or_default();
                            self.state_handles
                                .kill_buttons
                                .entry(session_key.clone())
                                .or_default();
                            live_session_keys.insert(session_key);
                            items.push(ListItem::Session {
                                host_key: key.clone(),
                                session_index: idx,
                            });
                        }
                        self.state_handles
                            .new_session_buttons
                            .entry(key.clone())
                            .or_default();
                        items.push(ListItem::NewSessionRow {
                            host_key: key.clone(),
                        });
                    }
                }
            }
        }

        self.state_handles
            .session_rows
            .retain(|k, _| live_session_keys.contains(k));
        self.state_handles
            .kill_buttons
            .retain(|k, _| live_session_keys.contains(k));

        self.list_items = Arc::new(items);
    }

    fn item_count(&self) -> usize {
        self.list_items.len()
    }
}

impl TypedActionView for RemoteSessionsPanelView {
    type Action = RemoteSessionsPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RemoteSessionsPanelAction::Refresh => {
                self.ingest_settings_into_model(ctx);
                self.rebuild_list_items(ctx);
                ctx.notify();
            }
            RemoteSessionsPanelAction::ConnectHost { key } => {
                let Some(host) = settings_hosts_snapshot(ctx)
                    .into_iter()
                    .find(|h| h.local_host_key == *key)
                else {
                    return;
                };
                let settings_handle = RemoteSessionsSettings::handle(ctx);
                let model_handle = self.model.clone();
                settings_handle.update(ctx, |settings, ctx| {
                    model_handle.update(ctx, |model, ctx| {
                        model.connect(host, settings, ctx);
                    });
                });
            }
            RemoteSessionsPanelAction::ToggleHostExpanded { key } => {
                if self.expanded_hosts.contains(key) {
                    self.expanded_hosts.remove(key);
                } else {
                    self.expanded_hosts.insert(key.clone());
                }
                self.rebuild_list_items(ctx);
                ctx.notify();
            }
            RemoteSessionsPanelAction::CreateDefaultSession { key } => {
                let host_key = key.clone();
                let existing: Vec<String> = self
                    .model
                    .as_ref(ctx)
                    .host_state(&host_key)
                    .map(|s| s.sessions.iter().map(|s| s.name.clone()).collect())
                    .unwrap_or_default();
                let name = next_default_session_name(&existing);
                self.model.update(ctx, |model, ctx| {
                    model.create_session(&host_key, name, None, ctx);
                });
                self.rebuild_list_items(ctx);
                ctx.notify();
            }
            RemoteSessionsPanelAction::AttachSession { key, name } => {
                ctx.dispatch_typed_action(&crate::workspace::WorkspaceAction::OpenRemoteAttachTab {
                    local_host_key: key.clone(),
                    session_name: name.clone(),
                });
            }
            RemoteSessionsPanelAction::KillSession { key, name } => {
                let key = key.clone();
                let name = name.clone();
                self.model.update(ctx, |model, ctx| {
                    model.kill_session(&key, name.clone(), ctx);
                });
                ctx.dispatch_typed_action(
                    &crate::workspace::WorkspaceAction::CloseRemoteAttachTab {
                        local_host_key: key,
                        session_name: name,
                    },
                );
            }
            RemoteSessionsPanelAction::OpenAddHostSettings => {
                ctx.dispatch_typed_action(&crate::workspace::WorkspaceAction::ShowSettingsPage(
                    crate::settings_view::SettingsSection::RemoteHosts,
                ));
            }
            RemoteSessionsPanelAction::DisconnectHost { key } => {
                let key = key.clone();
                self.model.update(ctx, |model, ctx| {
                    model.disconnect(&key, ctx);
                });
            }
            RemoteSessionsPanelAction::RequestRemoveHost { key } => {
                self.pending_remove = Some(key.clone());
                ctx.notify();
            }
            RemoteSessionsPanelAction::CancelRemoveHost => {
                self.pending_remove = None;
                ctx.notify();
            }
            RemoteSessionsPanelAction::ConfirmRemoveHost => {
                if let Some(key) = self.pending_remove.take() {
                    self.model.update(ctx, |model, ctx| {
                        model.disconnect(&key, ctx);
                    });
                    let settings_handle = RemoteSessionsSettings::handle(ctx);
                    settings_handle.update(ctx, |settings, ctx| {
                        settings.remove_host(&key, ctx);
                    });
                }
                ctx.notify();
            }
        }
    }
}

impl View for RemoteSessionsPanelView {
    fn ui_name() -> &'static str {
        "RemoteSessionsPanelView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let hosts = settings_hosts_snapshot(app);

        let header = render_header(self.state_handles.refresh_button.clone(), app);

        let content: Box<dyn Element> = if hosts.is_empty() {
            render_empty_state(self.state_handles.empty_state_button.clone(), app)
        } else {
            let list_items = self.list_items.clone();
            let hosts_by_key: std::collections::HashMap<String, RemoteHost> = hosts
                .iter()
                .map(|h| (h.local_host_key.clone(), h.clone()))
                .collect();
            let model_handle = self.model.downgrade();
            let expanded_hosts = self.expanded_hosts.clone();
            let pending_remove = self.pending_remove.clone();
            let host_states_clone = self.state_handles.host_rows.clone();
            let connect_states_clone = self.state_handles.connect_buttons.clone();
            let disconnect_states_clone = self.state_handles.disconnect_buttons.clone();
            let remove_states_clone = self.state_handles.remove_buttons.clone();
            let confirm_remove_state = self.state_handles.confirm_remove_button.clone();
            let cancel_remove_state = self.state_handles.cancel_remove_button.clone();
            let session_states_clone = self.state_handles.session_rows.clone();
            let kill_states_clone = self.state_handles.kill_buttons.clone();
            let new_session_button_states = self.state_handles.new_session_buttons.clone();

            let list = UniformList::new(
                self.state_handles.list_state.clone(),
                self.item_count(),
                move |range: Range<usize>, app: &AppContext| {
                    let Some(model) = model_handle.upgrade(app) else {
                        return Vec::<Box<dyn Element>>::new().into_iter();
                    };
                    let model = model.as_ref(app);

                    range
                        .filter_map(|index| {
                            let item = list_items.get(index)?;
                            match item {
                                ListItem::Host(key) => {
                                    let host = hosts_by_key.get(key)?;
                                    let state = model.host_state(key)?;
                                    let expanded = expanded_hosts.contains(key);
                                    let mouse_state = host_states_clone
                                        .get(key)
                                        .cloned()
                                        .unwrap_or_default();
                                    let connect_state = connect_states_clone
                                        .get(key)
                                        .cloned()
                                        .unwrap_or_default();
                                    let disconnect_state = disconnect_states_clone
                                        .get(key)
                                        .cloned()
                                        .unwrap_or_default();
                                    let remove_state = remove_states_clone
                                        .get(key)
                                        .cloned()
                                        .unwrap_or_default();
                                    let is_pending_remove =
                                        pending_remove.as_deref() == Some(key.as_str());
                                    Some(render_host_row(
                                        HostRowProps {
                                            state,
                                            host,
                                            expanded,
                                            mouse_state,
                                            connect_button_state: connect_state,
                                            disconnect_button_state: disconnect_state,
                                            remove_button_state: remove_state,
                                            confirm_remove_state: confirm_remove_state.clone(),
                                            cancel_remove_state: cancel_remove_state.clone(),
                                            is_pending_remove,
                                        },
                                        app,
                                    ))
                                }
                                ListItem::Session {
                                    host_key,
                                    session_index,
                                } => {
                                    let state = model.host_state(host_key)?;
                                    let session = state.sessions.get(*session_index)?;
                                    let session_key = format!("{host_key}::{}", session.name);
                                    let mouse_state = session_states_clone
                                        .get(&session_key)
                                        .cloned()
                                        .unwrap_or_default();
                                    let kill_state = kill_states_clone
                                        .get(&session_key)
                                        .cloned()
                                        .unwrap_or_default();
                                    Some(render_session_row(
                                        SessionRowProps {
                                            host_key: host_key.clone(),
                                            session,
                                            mouse_state,
                                            kill_button_state: kill_state,
                                        },
                                        app,
                                    ))
                                }
                                ListItem::NewSessionRow { host_key } => {
                                    let mouse_state = new_session_button_states
                                        .get(host_key)
                                        .cloned()
                                        .unwrap_or_default();
                                    Some(render_new_session_button(
                                        host_key,
                                        mouse_state,
                                        app,
                                    ))
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                },
            )
            .finish_scrollable();

            Scrollable::vertical(
                self.state_handles.scroll_state.clone(),
                list,
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish()
        };

        let footer = render_footer(self.state_handles.add_host_button.clone(), app);

        ConstrainedBox::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(header)
                .with_child(Shrinkable::new(1.0, content).finish())
                .with_child(footer)
                .finish(),
        )
        .with_width(PANEL_WIDTH)
        .finish()
    }
}

fn settings_hosts_snapshot(app: &AppContext) -> Vec<RemoteHost> {
    RemoteSessionsSettings::as_ref(app).hosts.to_vec()
}

fn next_default_session_name(existing: &[String]) -> String {
    let names: HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
    for i in 1..u32::MAX {
        let candidate = format!("session-{i}");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    "session-1".to_string()
}

fn render_header(
    refresh_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let title = Text::new_inline(
        "Remote sessions",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(theme.main_text_color(theme.background()).into())
    .finish();

    let refresh_icon = Hoverable::new(refresh_mouse_state, move |mouse_state| {
        let color = if mouse_state.is_hovered() {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        ConstrainedBox::new(Icon::Refresh.to_warpui_icon(color).finish())
            .with_width(REFRESH_ICON_SIZE)
            .with_height(REFRESH_ICON_SIZE)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(RemoteSessionsPanelAction::Refresh);
    })
    .finish();

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(Shrinkable::new(1.0, title).finish())
        .with_child(refresh_icon)
        .finish();

    Container::new(row)
        .with_horizontal_padding(PANEL_HEADER_HORIZONTAL_PADDING)
        .with_vertical_padding(PANEL_HEADER_VERTICAL_PADDING)
        .with_border(Border::bottom(1.).with_border_fill(theme.surface_3()))
        .finish()
}

fn render_footer(
    button_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size() - 1.;

    let button = Hoverable::new(button_mouse_state, move |mouse_state| {
        let label = Text::new_inline("+ Add remote host", font_family, font_size)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
        let mut container = Container::new(label)
            .with_horizontal_padding(8.)
            .with_vertical_padding(4.)
            .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if mouse_state.is_hovered() {
            container = container.with_background(theme.surface_3());
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(RemoteSessionsPanelAction::OpenAddHostSettings);
    })
    .finish();

    Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_child(button)
            .finish(),
    )
    .with_horizontal_padding(FOOTER_HORIZONTAL_PADDING)
    .with_vertical_padding(FOOTER_VERTICAL_PADDING)
    .with_border(Border::top(1.).with_border_fill(theme.surface_3()))
    .finish()
}

fn render_empty_state(
    button_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let icon = ConstrainedBox::new(
        Icon::RemoteServer
            .to_warpui_icon(theme.sub_text_color(theme.background()))
            .finish(),
    )
    .with_width(EMPTY_STATE_ICON_SIZE)
    .with_height(EMPTY_STATE_ICON_SIZE)
    .finish();

    let title = Text::new(
        "No remote hosts yet",
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_color(theme.sub_text_color(theme.background()).into_solid())
    .finish();

    let subtitle = ConstrainedBox::new(
        FormattedTextElement::from_str(
            "Add a remote SSH host to manage tmux sessions across machines.",
            appearance.ui_font_family(),
            appearance.ui_font_size() - 1.,
        )
        .with_alignment(TextAlignment::Center)
        .with_color(theme.disabled_ui_text_color().into_solid())
        .finish(),
    )
    .with_max_width(240.)
    .finish();

    let action_button = render_empty_state_action(button_mouse_state, appearance);

    let column = Flex::column()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(EMPTY_STATE_SPACING)
        .with_child(icon)
        .with_child(title)
        .with_child(subtitle)
        .with_child(action_button)
        .finish();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(column)
                .with_horizontal_padding(16.)
                .with_vertical_padding(24.)
                .finish(),
        )
        .finish()
}

fn render_empty_state_action(
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size() - 1.;
    Hoverable::new(mouse_state, move |mouse_state| {
        let label = Text::new_inline("Add remote host", font_family, font_size)
            .with_color(theme.main_text_color(theme.background()).into())
            .finish();
        let mut container = Container::new(label)
            .with_horizontal_padding(10.)
            .with_vertical_padding(4.)
            .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if mouse_state.is_hovered() {
            container = container.with_background(theme.surface_3());
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(RemoteSessionsPanelAction::OpenAddHostSettings);
    })
    .finish()
}

fn render_new_session_button(
    host_key: &str,
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.ui_font_size() - 1.;
    let host_key_for_click = host_key.to_string();
    Hoverable::new(mouse_state, move |mouse_state| {
        let color = if mouse_state.is_hovered() {
            theme.main_text_color(theme.background())
        } else {
            theme.sub_text_color(theme.background())
        };
        let label = Text::new_inline("+ New session", font_family, font_size)
            .with_color(color.into())
            .finish();
        Container::new(label)
            .with_padding_left(28.)
            .with_padding_right(12.)
            .with_vertical_padding(6.)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(RemoteSessionsPanelAction::CreateDefaultSession {
            key: host_key_for_click.clone(),
        });
    })
    .finish()
}

