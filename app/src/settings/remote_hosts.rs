use settings::{
    macros::{maybe_define_setting, register_settings_events},
    ChangeEventReason, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

#[derive(
    Default,
    Debug,
    Clone,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(rename_all = "snake_case")]
pub struct RemoteHost {
    pub local_host_key: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub identity_file: Option<String>,
    pub ssh_options: Vec<String>,
    pub created_at: i64,
}

impl RemoteHost {
    pub fn new_local_host_key() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn identity_file_arg(&self) -> Option<&str> {
        self.identity_file.as_deref().filter(|s| !s.is_empty())
    }
}

maybe_define_setting!(RemoteSessionsHosts, group: RemoteSessionsSettings, {
    type: Vec<RemoteHost>,
    default: Vec::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "remote_sessions.hosts",
    description: "SSH remote host configurations managed by the Remote Sessions panel.",
});

maybe_define_setting!(RemoteSessionsHeartbeatInterval, group: RemoteSessionsSettings, {
    type: u32,
    default: 30,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "remote_sessions.heartbeat_interval_seconds",
    description: "Heartbeat interval in seconds for the control plane connection.",
});

pub struct RemoteSessionsSettings {
    pub hosts: RemoteSessionsHosts,
    pub heartbeat_interval_seconds: RemoteSessionsHeartbeatInterval,
}

pub enum RemoteSessionsSettingsChangedEvent {
    RemoteSessionsHosts {
        change_event_reason: ChangeEventReason,
    },
    RemoteSessionsHeartbeatInterval {
        change_event_reason: ChangeEventReason,
    },
}

impl Entity for RemoteSessionsSettings {
    type Event = RemoteSessionsSettingsChangedEvent;
}

impl SingletonEntity for RemoteSessionsSettings {}

impl RemoteSessionsSettings {
    fn new_from_storage(ctx: &mut ModelContext<Self>) -> Self {
        Self {
            hosts: RemoteSessionsHosts::new_from_storage(ctx),
            heartbeat_interval_seconds: RemoteSessionsHeartbeatInterval::new_from_storage(ctx),
        }
    }

    pub fn register(ctx: &mut AppContext) {
        let handle = ctx.add_singleton_model(Self::new_from_storage);

        register_settings_events!(
            RemoteSessionsSettings,
            hosts,
            RemoteSessionsHosts,
            handle.clone(),
            ctx
        );
        register_settings_events!(
            RemoteSessionsSettings,
            heartbeat_interval_seconds,
            RemoteSessionsHeartbeatInterval,
            handle,
            ctx
        );
    }

    pub fn upsert_host(&mut self, host: RemoteHost, ctx: &mut ModelContext<Self>) {
        let mut list = self.hosts.to_vec();
        match list
            .iter()
            .position(|h| h.local_host_key == host.local_host_key)
        {
            Some(idx) => list[idx] = host,
            None => list.push(host),
        }
        self.hosts
            .set_value(list, ctx)
            .expect("remote_sessions.hosts failed to serialize");
        ctx.notify();
    }

    pub fn remove_host(&mut self, local_host_key: &str, ctx: &mut ModelContext<Self>) {
        let mut list = self.hosts.to_vec();
        list.retain(|h| h.local_host_key != local_host_key);
        self.hosts
            .set_value(list, ctx)
            .expect("remote_sessions.hosts failed to serialize");
        ctx.notify();
    }
}
