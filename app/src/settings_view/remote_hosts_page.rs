use chrono::Utc;
use settings::Setting as _;
use std::cell::RefCell;
use warpui::elements::{
    ChildView, Container, CornerRadius, CrossAxisAlignment, Element, Empty, Expanded, Flex,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::report_if_error;
use crate::settings::remote_hosts::{
    RemoteHost, RemoteSessionsSettings, RemoteSessionsSettingsChangedEvent,
};

use super::settings_page::{
    MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, HEADER_PADDING,
    SUBHEADER_FONT_SIZE,
};
use super::SettingsSection;

const PAGE_TITLE: &str = "Remote hosts";
const PAGE_DESCRIPTION: &str =
    "Configure SSH hosts to manage tmux sessions on them from Warp's Remote Sessions panel.";
const ADD_HOST_BUTTON_LABEL: &str = "+ Add host";
const ADVANCED_TITLE: &str = "Advanced";
const SECTION_FONT_SIZE: f32 = 14.;
const ROW_FONT_SIZE: f32 = 12.;
const DESCRIPTION_FONT_SIZE: f32 = 12.;
const ROW_PADDING: f32 = 8.;
const ROW_GAP: f32 = 4.;
const SECTION_GAP: f32 = 24.;
const FIELD_LABEL_FONT_SIZE: f32 = 12.;
const FIELD_GAP: f32 = 12.;
const MODAL_WIDTH: f32 = 520.;
const MODAL_HEIGHT: f32 = 560.;
const DEFAULT_PORT: u16 = 22;
const MIN_HEARTBEAT: u32 = 10;
const MAX_HEARTBEAT: u32 = 120;

pub struct RemoteHostsPageView {
    page: PageType<Self>,
    add_host_button_state: MouseStateHandle,
    row_button_states: RefCell<Vec<RowButtonStates>>,
    heartbeat_editor: ViewHandle<EditorView>,
    modal_state: HostModalState,
    modal_view: ViewHandle<Modal<HostFormView>>,
    editing_local_host_key: Option<String>,
    pending_remove: Option<String>,
    confirm_remove_state: MouseStateHandle,
    cancel_remove_state: MouseStateHandle,
}

#[derive(Default)]
struct RowButtonStates {
    edit: MouseStateHandle,
    remove: MouseStateHandle,
}

struct HostModalState {
    state: ModalViewState<Modal<HostFormView>>,
}

impl HostModalState {
    fn new(view: ViewHandle<Modal<HostFormView>>) -> Self {
        Self {
            state: ModalViewState::new(view),
        }
    }

    fn is_open(&self) -> bool {
        self.state.is_open()
    }

    fn render(&self) -> Box<dyn Element> {
        self.state.render()
    }

    fn open<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.open();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_open(ctx);
            });
        });
    }

    fn close<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.close();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_close(ctx);
            });
        });
    }
}

#[derive(Clone, Debug)]
pub enum RemoteHostsPageAction {
    ShowAddModal,
    ShowEditModal(String),
    RequestRemoveHost(String),
    ConfirmRemoveHost,
    CancelRemoveHost,
}

#[derive(PartialEq, Eq)]
pub enum RemoteHostsPageEvent {
    FocusModal,
    ShowModal,
    HideModal,
}

impl RemoteHostsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let settings_handle = RemoteSessionsSettings::handle(ctx);
        ctx.observe(&settings_handle, |me, _, ctx| {
            me.refresh_row_states(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&settings_handle, |me, _, event, ctx| {
            if matches!(
                event,
                RemoteSessionsSettingsChangedEvent::RemoteSessionsHosts { .. }
            ) {
                me.refresh_row_states(ctx);
            }
            me.sync_numeric_editors(ctx);
            ctx.notify();
        });

        let form_body = ctx.add_typed_action_view(HostFormView::new);
        ctx.subscribe_to_view(&form_body, |me, _, event, ctx| {
            me.handle_form_event(event, ctx);
        });

        let modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some("Add remote host".to_string()), form_body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(MODAL_WIDTH),
                    height: Some(MODAL_HEIGHT),
                    ..Default::default()
                })
                .with_header_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 24.,
                        bottom: 0.,
                        left: 24.,
                        right: 24.,
                    }),
                    font_size: Some(16.),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 0.,
                        bottom: 24.,
                        left: 24.,
                        right: 24.,
                    }),
                    ..Default::default()
                })
                .with_background_opacity(100)
                .with_dismiss_on_click()
        });
        ctx.subscribe_to_view(&modal_view, |me, _, event, ctx| {
            me.handle_modal_event(event, ctx);
        });

        let heartbeat_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        ctx.subscribe_to_view(&heartbeat_editor, |me, _, event, ctx| {
            me.handle_heartbeat_editor_event(event, ctx);
        });

        let mut view = Self {
            page: PageType::new_monolith(RemoteHostsWidget, None, true),
            add_host_button_state: MouseStateHandle::default(),
            row_button_states: RefCell::new(Vec::new()),
            heartbeat_editor,
            modal_state: HostModalState::new(modal_view.clone()),
            modal_view,
            editing_local_host_key: None,
            pending_remove: None,
            confirm_remove_state: MouseStateHandle::default(),
            cancel_remove_state: MouseStateHandle::default(),
        };
        view.refresh_row_states(ctx);
        view.sync_numeric_editors(ctx);
        view
    }

    fn refresh_row_states(&mut self, ctx: &mut ViewContext<Self>) {
        let hosts_len = RemoteSessionsSettings::as_ref(ctx).hosts.to_vec().len();
        let mut states = self.row_button_states.borrow_mut();
        states.resize_with(hosts_len, RowButtonStates::default);
    }

    fn sync_numeric_editors(&mut self, ctx: &mut ViewContext<Self>) {
        let heartbeat = *RemoteSessionsSettings::as_ref(ctx)
            .heartbeat_interval_seconds
            .value();
        let heartbeat_text = heartbeat.to_string();
        self.heartbeat_editor.update(ctx, |editor, ctx| {
            if editor.buffer_text(ctx) != heartbeat_text {
                editor.set_buffer_text(&heartbeat_text, ctx);
            }
        });
    }

    fn open_add_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.editing_local_host_key = None;
        self.modal_view.update(ctx, |modal, _| {
            modal.set_title(Some("Add remote host".to_string()));
        });
        self.modal_view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.load(None, ctx);
            });
        });
        self.modal_state.open(ctx);
        ctx.emit(RemoteHostsPageEvent::ShowModal);
        ctx.notify();
    }

    fn open_edit_modal(&mut self, local_host_key: String, ctx: &mut ViewContext<Self>) {
        let host = RemoteSessionsSettings::as_ref(ctx)
            .hosts
            .iter()
            .find(|h| h.local_host_key == local_host_key)
            .cloned();
        let Some(host) = host else {
            return;
        };
        self.editing_local_host_key = Some(local_host_key);
        self.modal_view.update(ctx, |modal, _| {
            modal.set_title(Some("Edit remote host".to_string()));
        });
        self.modal_view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.load(Some(host), ctx);
            });
        });
        self.modal_state.open(ctx);
        ctx.emit(RemoteHostsPageEvent::ShowModal);
        ctx.notify();
    }

    fn close_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.modal_state.close(ctx);
        self.editing_local_host_key = None;
        ctx.emit(RemoteHostsPageEvent::HideModal);
        ctx.notify();
    }

    fn handle_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => self.close_modal(ctx),
        }
    }

    fn handle_form_event(&mut self, event: &HostFormEvent, ctx: &mut ViewContext<Self>) {
        match event {
            HostFormEvent::Cancel => self.close_modal(ctx),
            HostFormEvent::Submit(submission) => {
                self.save_host(submission.clone(), ctx);
            }
        }
    }

    fn save_host(&mut self, submission: HostSubmission, ctx: &mut ViewContext<Self>) {
        let existing_key = self.editing_local_host_key.clone();
        let existing_created_at = existing_key.as_ref().and_then(|key| {
            RemoteSessionsSettings::as_ref(ctx)
                .hosts
                .iter()
                .find(|h| &h.local_host_key == key)
                .map(|h| h.created_at)
        });

        let host = RemoteHost {
            local_host_key: existing_key.unwrap_or_else(RemoteHost::new_local_host_key),
            alias: submission.alias,
            host: submission.host,
            port: submission.port,
            identity_file: submission.identity_file,
            ssh_options: submission.ssh_options,
            created_at: existing_created_at.unwrap_or_else(|| Utc::now().timestamp()),
        };

        RemoteSessionsSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.upsert_host(host, ctx);
        });
        self.close_modal(ctx);
    }

    fn request_remove_host(&mut self, local_host_key: String, ctx: &mut ViewContext<Self>) {
        self.pending_remove = Some(local_host_key);
        ctx.notify();
    }

    fn cancel_remove_host(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_remove = None;
        ctx.notify();
    }

    fn confirm_remove_host(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(key) = self.pending_remove.take() else {
            return;
        };
        RemoteSessionsSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.remove_host(&key, ctx);
        });
        ctx.notify();
    }

    fn handle_heartbeat_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                let text = self.heartbeat_editor.as_ref(ctx).buffer_text(ctx);
                if let Ok(value) = text.parse::<u32>() {
                    let clamped = value.clamp(MIN_HEARTBEAT, MAX_HEARTBEAT);
                    let handle = RemoteSessionsSettings::handle(ctx);
                    if *handle.as_ref(ctx).heartbeat_interval_seconds.value() != clamped {
                        handle.update(ctx, |settings, ctx| {
                            report_if_error!(settings
                                .heartbeat_interval_seconds
                                .set_value(clamped, ctx));
                        });
                    }
                }
            }
            EditorEvent::Escape => ctx.emit(RemoteHostsPageEvent::FocusModal),
            _ => {}
        }
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        if self.modal_state.is_open() {
            return Some(self.modal_state.render());
        }
        None
    }
}

impl Entity for RemoteHostsPageView {
    type Event = RemoteHostsPageEvent;
}

impl View for RemoteHostsPageView {
    fn ui_name() -> &'static str {
        "RemoteHostsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for RemoteHostsPageView {
    fn section() -> SettingsSection {
        SettingsSection::RemoteHosts
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl TypedActionView for RemoteHostsPageView {
    type Action = RemoteHostsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RemoteHostsPageAction::ShowAddModal => self.open_add_modal(ctx),
            RemoteHostsPageAction::ShowEditModal(key) => self.open_edit_modal(key.clone(), ctx),
            RemoteHostsPageAction::RequestRemoveHost(key) => {
                self.request_remove_host(key.clone(), ctx)
            }
            RemoteHostsPageAction::CancelRemoveHost => self.cancel_remove_host(ctx),
            RemoteHostsPageAction::ConfirmRemoveHost => self.confirm_remove_host(ctx),
        }
    }
}

impl From<ViewHandle<RemoteHostsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<RemoteHostsPageView>) -> Self {
        SettingsPageViewHandle::RemoteHosts(view_handle)
    }
}

struct RemoteHostsWidget;

impl SettingsWidget for RemoteHostsWidget {
    type View = RemoteHostsPageView;

    fn search_terms(&self) -> &str {
        "remote hosts ssh tmux sessions heartbeat"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.0,
                    Flex::column()
                        .with_child(
                            Text::new_inline(
                                PAGE_TITLE,
                                appearance.ui_font_family(),
                                SUBHEADER_FONT_SIZE + 6.,
                            )
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                Text::new(
                                    PAGE_DESCRIPTION,
                                    appearance.ui_font_family(),
                                    DESCRIPTION_FONT_SIZE,
                                )
                                .with_color(theme.sub_text_color(theme.background()).into())
                                .finish(),
                            )
                            .with_margin_top(6.)
                            .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                ui_builder
                    .button(ButtonVariant::Accent, view.add_host_button_state.clone())
                    .with_text_label(ADD_HOST_BUTTON_LABEL.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        padding: Some(Coords::uniform(8.).left(12.).right(12.)),
                        ..Default::default()
                    })
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(RemoteHostsPageAction::ShowAddModal);
                    })
                    .finish(),
            )
            .finish();

        let hosts = RemoteSessionsSettings::as_ref(app).hosts.to_vec();
        let row_states = view.row_button_states.borrow();
        let pending_remove = view.pending_remove.clone();

        let mut hosts_list = Flex::column();
        if hosts.is_empty() {
            hosts_list.add_child(
                Container::new(
                    Text::new(
                        "No remote hosts configured yet.",
                        appearance.ui_font_family(),
                        ROW_FONT_SIZE,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_uniform_padding(ROW_PADDING)
                .finish(),
            );
        } else {
            for (idx, host) in hosts.iter().enumerate() {
                let edit_state = row_states
                    .get(idx)
                    .map(|s| s.edit.clone())
                    .unwrap_or_default();
                let remove_state = row_states
                    .get(idx)
                    .map(|s| s.remove.clone())
                    .unwrap_or_default();
                let is_confirming = pending_remove.as_deref() == Some(host.local_host_key.as_str());
                hosts_list.add_child(render_host_row(
                    host,
                    edit_state,
                    remove_state,
                    is_confirming,
                    view.confirm_remove_state.clone(),
                    view.cancel_remove_state.clone(),
                    appearance,
                ));
            }
        }

        let advanced_section = render_advanced_section(view, appearance);

        Flex::column()
            .with_child(
                Container::new(header_row)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            )
            .with_child(
                Container::new(hosts_list.finish())
                    .with_padding_bottom(SECTION_GAP)
                    .finish(),
            )
            .with_child(advanced_section)
            .finish()
    }
}

fn render_host_row(
    host: &RemoteHost,
    edit_state: MouseStateHandle,
    remove_state: MouseStateHandle,
    is_confirming_remove: bool,
    confirm_state: MouseStateHandle,
    cancel_state: MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();

    let host_target = if host.port == DEFAULT_PORT {
        host.host.clone()
    } else {
        format!("{}:{}", host.host, host.port)
    };

    let mut details = Flex::column().with_child(
        Text::new_inline(
            host.alias.clone(),
            appearance.ui_font_family(),
            ROW_FONT_SIZE + 1.,
        )
        .with_color(theme.active_ui_text_color().into())
        .finish(),
    );
    details.add_child(
        Container::new(
            Text::new_inline(host_target, appearance.ui_font_family(), ROW_FONT_SIZE)
                .with_color(theme.sub_text_color(theme.surface_overlay_1()).into())
                .finish(),
        )
        .with_margin_top(2.)
        .finish(),
    );
    if let Some(identity) = host.identity_file_arg() {
        details.add_child(
            Container::new(
                Text::new_inline(
                    identity.to_owned(),
                    appearance.ui_font_family(),
                    ROW_FONT_SIZE,
                )
                .with_color(theme.sub_text_color(theme.surface_overlay_1()).into())
                .finish(),
            )
            .with_margin_top(2.)
            .finish(),
        );
    }

    let local_host_key_edit = host.local_host_key.clone();
    let local_host_key_remove = host.local_host_key.clone();

    let button_style = UiComponentStyles {
        font_size: Some(11.),
        padding: Some(Coords::uniform(6.).left(10.).right(10.)),
        ..Default::default()
    };

    let actions = if is_confirming_remove {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    Text::new_inline(
                        "Remove this host?".to_string(),
                        appearance.ui_font_family(),
                        ROW_FONT_SIZE,
                    )
                    .with_color(theme.ui_error_color().into())
                    .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                ui_builder
                    .button(ButtonVariant::Secondary, cancel_state)
                    .with_text_label("Cancel".to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(RemoteHostsPageAction::CancelRemoveHost);
                    })
                    .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .button(ButtonVariant::Accent, confirm_state)
                        .with_text_label("Confirm".to_string())
                        .with_style(button_style)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(RemoteHostsPageAction::ConfirmRemoveHost);
                        })
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .finish()
    } else {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                ui_builder
                    .button(ButtonVariant::Secondary, edit_state)
                    .with_text_label("Edit".to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(RemoteHostsPageAction::ShowEditModal(
                            local_host_key_edit.clone(),
                        ));
                    })
                    .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .button(ButtonVariant::Secondary, remove_state)
                        .with_text_label("Remove".to_string())
                        .with_style(button_style)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(RemoteHostsPageAction::RequestRemoveHost(
                                local_host_key_remove.clone(),
                            ));
                        })
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            )
            .finish()
    };

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Expanded::new(1., details.finish()).finish())
            .with_child(actions)
            .finish(),
    )
    .with_background(theme.surface_overlay_1())
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
    .with_uniform_padding(ROW_PADDING + 2.)
    .with_margin_bottom(ROW_GAP)
    .finish()
}

fn render_advanced_section(
    view: &RemoteHostsPageView,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let header = Container::new(
        Text::new_inline(
            ADVANCED_TITLE,
            appearance.ui_font_family(),
            SECTION_FONT_SIZE,
        )
        .with_color(theme.active_ui_text_color().into())
        .finish(),
    )
    .with_margin_bottom(8.)
    .finish();

    let heartbeat_row = render_numeric_row(
        "Heartbeat interval (seconds)",
        &format!(
            "Control plane keep-alive interval. Press Enter to apply. Range {}-{}.",
            MIN_HEARTBEAT, MAX_HEARTBEAT
        ),
        &view.heartbeat_editor,
        appearance,
    );

    Flex::column()
        .with_child(header)
        .with_child(heartbeat_row)
        .finish()
}

fn render_numeric_row(
    title: &str,
    description: &str,
    editor: &ViewHandle<EditorView>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();

    let label_column = Flex::column()
        .with_child(
            Text::new_inline(
                title.to_string(),
                appearance.ui_font_family(),
                ROW_FONT_SIZE + 1.,
            )
            .with_color(theme.active_ui_text_color().into())
            .finish(),
        )
        .with_child(
            Container::new(
                Text::new(
                    description.to_string(),
                    appearance.ui_font_family(),
                    DESCRIPTION_FONT_SIZE,
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_margin_top(2.)
            .finish(),
        )
        .finish();

    let editor_input = ui_builder
        .text_input(editor.clone())
        .with_style(UiComponentStyles {
            width: Some(96.),
            ..Default::default()
        })
        .build()
        .finish();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Expanded::new(1., label_column).finish())
            .with_child(editor_input)
            .finish(),
    )
    .with_padding_bottom(HEADER_PADDING)
    .finish()
}

#[derive(Clone, Debug)]
pub struct HostSubmission {
    alias: String,
    host: String,
    port: u16,
    identity_file: Option<String>,
    ssh_options: Vec<String>,
}

#[derive(Debug)]
pub enum HostFormEvent {
    Cancel,
    Submit(HostSubmission),
}

#[derive(Debug)]
pub enum HostFormAction {
    Cancel,
    Submit,
    TestConnection,
}

#[derive(Clone, Debug, Default)]
pub enum TestConnectionState {
    #[default]
    Idle,
    Running,
    Ok(String),
    Err(String),
}

pub struct HostFormView {
    alias_editor: ViewHandle<EditorView>,
    host_editor: ViewHandle<EditorView>,
    port_editor: ViewHandle<EditorView>,
    identity_editor: ViewHandle<EditorView>,
    ssh_options_editor: ViewHandle<EditorView>,
    cancel_button_state: MouseStateHandle,
    save_button_state: MouseStateHandle,
    test_button_state: MouseStateHandle,
    validation_error: Option<String>,
    test_state: TestConnectionState,
}

impl HostFormView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();

        let make_editor = |placeholder: &'static str, ctx: &mut ViewContext<Self>| {
            ctx.add_typed_action_view(move |ctx| {
                let options = SingleLineEditorOptions {
                    text: TextOptions {
                        font_family_override: Some(font_family),
                        ..Default::default()
                    },
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                };
                let mut editor = EditorView::single_line(options, ctx);
                editor.set_placeholder_text(placeholder, ctx);
                editor
            })
        };

        let alias_editor = make_editor("e.g. \"prod-app-1\"", ctx);
        let host_editor = make_editor("user@host or host", ctx);
        let port_editor = make_editor("22", ctx);
        let identity_editor = make_editor("~/.ssh/id_ed25519 (optional)", ctx);
        let ssh_options_editor = make_editor(
            "Optional SSH options, comma separated (e.g. ProxyJump=bastion)",
            ctx,
        );

        for handle in [
            &alias_editor,
            &host_editor,
            &port_editor,
            &identity_editor,
            &ssh_options_editor,
        ] {
            ctx.subscribe_to_view(handle, |me, _, event, ctx| {
                me.handle_editor_event(event, ctx);
            });
        }

        Self {
            alias_editor,
            host_editor,
            port_editor,
            identity_editor,
            ssh_options_editor,
            cancel_button_state: MouseStateHandle::default(),
            save_button_state: MouseStateHandle::default(),
            test_button_state: MouseStateHandle::default(),
            validation_error: None,
            test_state: TestConnectionState::Idle,
        }
    }

    fn build_submission(&self, ctx: &ViewContext<Self>) -> Result<HostSubmission, String> {
        let alias = self
            .alias_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        let host = self
            .host_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        let port_text = self
            .port_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        let identity = self
            .identity_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        let options_text = self.ssh_options_editor.as_ref(ctx).buffer_text(ctx);

        if alias.is_empty() {
            return Err("Alias is required.".to_string());
        }
        if host.is_empty() {
            return Err("Host is required.".to_string());
        }
        let port = if port_text.is_empty() {
            DEFAULT_PORT
        } else {
            match port_text.parse::<u16>() {
                Ok(p) if p > 0 => p,
                _ => return Err("Port must be a number between 1 and 65535.".to_string()),
            }
        };
        let ssh_options: Vec<String> = options_text
            .split(|c: char| c == ',' || c == '\n')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let identity_file = if identity.is_empty() {
            None
        } else {
            Some(identity)
        };
        Ok(HostSubmission {
            alias,
            host,
            port,
            identity_file,
            ssh_options,
        })
    }

    #[cfg(feature = "remote_sessions")]
    fn test_connection(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(self.test_state, TestConnectionState::Running) {
            return;
        }
        let submission = match self.build_submission(ctx) {
            Ok(s) => s,
            Err(msg) => {
                self.validation_error = Some(msg);
                ctx.notify();
                return;
            }
        };
        let host = RemoteHost {
            local_host_key: String::new(),
            alias: submission.alias,
            host: submission.host,
            port: submission.port,
            identity_file: submission.identity_file,
            ssh_options: submission.ssh_options,
            created_at: 0,
        };
        self.test_state = TestConnectionState::Running;
        self.validation_error = None;
        ctx.notify();
        ctx.spawn(
            async move { crate::terminal::remote_sessions::probe::probe_host(&host).await },
            |me, result, ctx| {
                me.test_state = match result {
                    Ok(caps) => TestConnectionState::Ok(format!(
                        "tmux {} on {}",
                        caps.tmux_version, caps.os
                    )),
                    Err(e) => TestConnectionState::Err(e.to_string()),
                };
                ctx.notify();
            },
        );
    }

    #[cfg(not(feature = "remote_sessions"))]
    fn test_connection(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => self.submit(ctx),
            EditorEvent::Escape => self.cancel(ctx),
            EditorEvent::Edited(_) => {
                if self.validation_error.is_some() {
                    self.validation_error = None;
                    ctx.notify();
                }
            }
            _ => {}
        }
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        match self.build_submission(ctx) {
            Ok(submission) => {
                self.validation_error = None;
                ctx.emit(HostFormEvent::Submit(submission));
            }
            Err(msg) => {
                if msg.starts_with("Alias") {
                    ctx.focus(&self.alias_editor);
                } else if msg.starts_with("Host") {
                    ctx.focus(&self.host_editor);
                } else if msg.starts_with("Port") {
                    ctx.focus(&self.port_editor);
                }
                self.validation_error = Some(msg);
                ctx.notify();
            }
        }
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(HostFormEvent::Cancel);
    }

    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.alias_editor);
    }

    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_editors(ctx);
        self.validation_error = None;
        self.test_state = TestConnectionState::Idle;
    }

    pub fn load(&mut self, host: Option<RemoteHost>, ctx: &mut ViewContext<Self>) {
        self.clear_editors(ctx);
        self.validation_error = None;
        self.test_state = TestConnectionState::Idle;
        if let Some(host) = host {
            set_editor_text(&self.alias_editor, &host.alias, ctx);
            set_editor_text(&self.host_editor, &host.host, ctx);
            set_editor_text(&self.port_editor, &host.port.to_string(), ctx);
            if let Some(identity) = host.identity_file.as_ref() {
                set_editor_text(&self.identity_editor, identity, ctx);
            }
            if !host.ssh_options.is_empty() {
                set_editor_text(&self.ssh_options_editor, &host.ssh_options.join(", "), ctx);
            }
        } else {
            set_editor_text(&self.port_editor, &DEFAULT_PORT.to_string(), ctx);
        }
        ctx.focus(&self.alias_editor);
    }

    fn clear_editors(&mut self, ctx: &mut ViewContext<Self>) {
        for handle in [
            &self.alias_editor,
            &self.host_editor,
            &self.port_editor,
            &self.identity_editor,
            &self.ssh_options_editor,
        ] {
            handle.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
        }
    }
}

fn set_editor_text(
    handle: &ViewHandle<EditorView>,
    value: &str,
    ctx: &mut ViewContext<HostFormView>,
) {
    handle.update(ctx, |editor, ctx| {
        editor.set_buffer_text(value, ctx);
    });
}

impl Entity for HostFormView {
    type Event = HostFormEvent;
}

impl TypedActionView for HostFormView {
    type Action = HostFormAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HostFormAction::Cancel => self.cancel(ctx),
            HostFormAction::Submit => self.submit(ctx),
            HostFormAction::TestConnection => self.test_connection(ctx),
        }
    }
}

impl View for HostFormView {
    fn ui_name() -> &'static str {
        "HostFormView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let label = |text: &str, appearance: &Appearance| -> Box<dyn Element> {
            Container::new(
                Text::new_inline(
                    text.to_string(),
                    appearance.ui_font_family(),
                    FIELD_LABEL_FONT_SIZE,
                )
                .with_color(theme.active_ui_text_color().into())
                .finish(),
            )
            .with_margin_bottom(4.)
            .finish()
        };

        let field = |label_text: &str,
                     editor: &ViewHandle<EditorView>,
                     appearance: &Appearance|
         -> Box<dyn Element> {
            Container::new(
                Flex::column()
                    .with_child(label(label_text, appearance))
                    .with_child(ChildView::new(editor).finish())
                    .finish(),
            )
            .with_margin_bottom(FIELD_GAP)
            .finish()
        };

        let button_style = UiComponentStyles {
            font_size: Some(13.),
            padding: Some(Coords::uniform(8.).left(14.).right(14.)),
            ..Default::default()
        };

        let cancel_button = ui_builder
            .button(ButtonVariant::Secondary, self.cancel_button_state.clone())
            .with_text_label("Cancel".to_string())
            .with_style(button_style)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(HostFormAction::Cancel);
            })
            .finish();

        let test_label = if matches!(self.test_state, TestConnectionState::Running) {
            "Testing…".to_string()
        } else {
            "Test connection".to_string()
        };
        let test_button = ui_builder
            .button(ButtonVariant::Secondary, self.test_button_state.clone())
            .with_text_label(test_label)
            .with_style(button_style)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(HostFormAction::TestConnection);
            })
            .finish();

        let save_button = ui_builder
            .button(ButtonVariant::Accent, self.save_button_state.clone())
            .with_text_label("Save".to_string())
            .with_style(button_style)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(HostFormAction::Submit);
            })
            .finish();

        let feedback_element: Box<dyn Element> = if let Some(msg) = &self.validation_error {
            Container::new(
                Text::new(
                    msg.clone(),
                    appearance.ui_font_family(),
                    FIELD_LABEL_FONT_SIZE,
                )
                .with_color(theme.ui_error_color().into())
                .finish(),
            )
            .with_margin_bottom(8.)
            .finish()
        } else {
            match &self.test_state {
                TestConnectionState::Idle => Empty::new().finish(),
                TestConnectionState::Running => Container::new(
                    Text::new(
                        "Testing connection…".to_string(),
                        appearance.ui_font_family(),
                        FIELD_LABEL_FONT_SIZE,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
                TestConnectionState::Ok(detail) => Container::new(
                    Text::new(
                        format!("Connection OK · {detail}"),
                        appearance.ui_font_family(),
                        FIELD_LABEL_FONT_SIZE,
                    )
                    .with_color(theme.ansi_fg_green().into())
                    .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
                TestConnectionState::Err(detail) => Container::new(
                    Text::new(
                        format!("Connection failed: {detail}"),
                        appearance.ui_font_family(),
                        FIELD_LABEL_FONT_SIZE,
                    )
                    .with_color(theme.ui_error_color().into())
                    .finish(),
                )
                .with_margin_bottom(8.)
                .finish(),
            }
        };

        let buttons_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(test_button)
            .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
            .with_child(cancel_button)
            .with_child(Container::new(save_button).with_margin_left(12.).finish())
            .finish();

        Flex::column()
            .with_child(field("Alias", &self.alias_editor, appearance))
            .with_child(field("Host", &self.host_editor, appearance))
            .with_child(field("Port", &self.port_editor, appearance))
            .with_child(field("Identity file", &self.identity_editor, appearance))
            .with_child(field("SSH options", &self.ssh_options_editor, appearance))
            .with_child(feedback_element)
            .with_child(buttons_row)
            .finish()
    }
}
