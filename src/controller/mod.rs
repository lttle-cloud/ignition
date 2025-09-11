pub mod context;
pub mod scheduler;

pub mod app;
pub mod certificate;
pub mod machine;
pub mod service;
pub mod volume;

use anyhow::Result;
use async_trait::async_trait;
use std::{sync::Arc, time::Duration};

use crate::{
    agent::Agent,
    controller::context::{ControllerContext, ControllerEvent, ControllerKey},
    repository::Repository,
    resources::metadata::Metadata,
};

pub enum ReconcileNext {
    Done,
    Immediate,
    After(Duration),
}

impl ReconcileNext {
    pub fn done() -> Self {
        Self::Done
    }

    pub fn immediate() -> Self {
        Self::Immediate
    }

    pub fn after(duration: Duration) -> Self {
        Self::After(duration)
    }
}

#[async_trait]
pub trait Controller: Send + Sync {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>>;

    async fn should_reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> bool;

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext>;

    async fn handle_error(
        &self,
        ctx: ControllerContext,
        key: ControllerKey,
        error: anyhow::Error,
    ) -> ReconcileNext;
}

#[async_trait]
pub trait AdmissionCheckBeforeSet {
    async fn before_set(
        &self,
        before: Option<&Self>,
        tenant: String,
        repo: Arc<Repository>,
        agent: Arc<Agent>,
        metadata: Metadata,
    ) -> Result<()>;
}

#[async_trait]
pub trait AdmissionCheckBeforeDelete {
    async fn before_delete(
        &self,
        tenant: String,
        repo: Arc<Repository>,
        agent: Arc<Agent>,
        metadata: Metadata,
    ) -> Result<()>;
}
