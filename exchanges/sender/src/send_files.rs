// TODO: move interactors to different modules

mod handler;

use crate::{SenderError, SenderFile, SenderFileDataAdapter, SenderProfile};
use chrono::{DateTime, Utc};
use entities::{File, Profile};
use iroh::{Endpoint, protocol::Router};
use iroh_base::ticket::NodeTicket;
use rand::Rng;
use std::{fmt::Debug, sync::Arc};
use uuid::Uuid;

pub use handler::*;

pub struct SendFilesRequest {
    pub profile: SenderProfile,
    pub files: Vec<SenderFile>,
}

#[derive(Debug)]
pub struct SendFilesBubble {
    ticket: String,
    confirmation: u8,
    router: Arc<Router>,
    handler: Arc<SendFilesHandler>,
    created_at: DateTime<Utc>,
}
impl SendFilesBubble {
    pub fn new(
        ticket: String,
        confirmation: u8,
        router: Arc<Router>,
        handler: Arc<SendFilesHandler>,
    ) -> Self {
        return Self {
            ticket,
            confirmation,
            router,
            handler,
            created_at: Utc::now(),
        };
    }

    pub fn get_ticket(&self) -> String {
        return self.ticket.clone();
    }

    pub fn get_confirmation(&self) -> u8 {
        return self.confirmation;
    }

    pub async fn cancel(&self) -> () {
        // TODO: HANDLE FAILED AWAIT
        let _ = self.router.shutdown().await;
        return ();
    }

    pub fn is_finished(&self) -> bool {
        return self.router.is_shutdown() || self.handler.is_finished();
    }

    pub fn is_connected(&self) -> bool {
        if self.is_finished() {
            return false;
        }
        return self.handler.is_consumed();
    }

    pub fn get_created_at(&self) -> String {
        return self.created_at.to_rfc3339();
    }

    pub fn subscribe(&self, subscriber: Arc<dyn SendFilesSubscriber>) {
        return self.handler.subscribe(subscriber);
    }

    pub fn unsubscribe(&self, subscriber: Arc<dyn SendFilesSubscriber>) {
        return self.handler.unsubscribe(subscriber);
    }
}

pub async fn send_files(request: SendFilesRequest) -> Result<Arc<SendFilesBubble>, SenderError> {
    let endpoint = Endpoint::builder()
        .discovery_n0()
        .bind()
        .await
        .map_err(|e| SenderError::TODO(e.to_string()))?;
    let node_addr = endpoint
        .node_addr()
        .await
        .map_err(|e| SenderError::TODO(e.to_string()))?;
    let confirmation: u8 = rand::rng().random_range(0..=99);
    let handler = Arc::new(SendFilesHandler::new(
        Profile {
            id: Uuid::new_v4().to_string(),
            name: request.profile.name,
        },
        request
            .files
            .into_iter()
            .map(|f| File {
                id: Uuid::new_v4().to_string(),
                name: f.name,
                data: Arc::new(SenderFileDataAdapter { data: f.data }),
            })
            .collect(),
    ));
    let router = Arc::new(
        Router::builder(endpoint)
            .accept([confirmation], handler.clone())
            .spawn()
            .await
            .map_err(|e| SenderError::TODO(e.to_string()))?,
    );
    return Ok(Arc::new(SendFilesBubble::new(
        NodeTicket::new(node_addr).to_string(),
        confirmation,
        router,
        handler,
    )));
}
