use std::collections::HashMap;

use warp_core::SessionId;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

#[derive(Clone, Debug)]
pub struct RemoteAttachInfo {
    pub local_host_key: String,
    pub session_name: String,
}

#[derive(Default)]
pub struct RemoteAttachRegistry {
    by_session: HashMap<SessionId, RemoteAttachInfo>,
}

impl Entity for RemoteAttachRegistry {
    type Event = ();
}

impl SingletonEntity for RemoteAttachRegistry {}

impl RemoteAttachRegistry {
    pub fn register(ctx: &mut AppContext) {
        let handle: ModelHandle<Self> = ctx.add_singleton_model(|_| Self::default());
        let _ = handle;
    }

    pub fn record_if_absent(
        &mut self,
        session_id: SessionId,
        info: RemoteAttachInfo,
        _ctx: &mut ModelContext<Self>,
    ) {
        self.by_session.entry(session_id).or_insert(info);
    }

    pub fn forget_session(&mut self, session_id: SessionId, _ctx: &mut ModelContext<Self>) {
        self.by_session.remove(&session_id);
    }

    pub fn forget_host(&mut self, local_host_key: &str, _ctx: &mut ModelContext<Self>) {
        self.by_session
            .retain(|_, info| info.local_host_key != local_host_key);
    }

    pub fn lookup(&self, session_id: SessionId) -> Option<&RemoteAttachInfo> {
        self.by_session.get(&session_id)
    }
}
