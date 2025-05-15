// TODO: move interactors to different modules

mod handler;

use chrono::{DateTime, Utc};
use entities::{Data, File, Profile};
use handler::send_files::{SendFilesHandler, SendFilesHandlerResources};
use iroh::{Endpoint, protocol::Router};
use iroh_base::ticket::NodeTicket;
use rand::Rng;
use std::{fmt::Debug, sync::Arc};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error("TODO: \"{0}\".")]
    TODO(String),
}

// REQUEST ->
pub struct SendFilesRequest {
    pub profile: SenderProfile,
    pub files: Vec<SenderFile>,
}

pub struct SenderProfile {
    pub name: String,
}

pub struct SenderFile {
    pub name: String,
    pub data: Arc<dyn SenderFileData>,
}

pub trait SenderFileData: Send + Sync {
    fn len(&self) -> u64;
    fn read(&self) -> Option<u8>;
}
struct SenderFileDataAdapter {
    data: Arc<dyn SenderFileData>,
}
impl Data for SenderFileDataAdapter {
    fn len(&self) -> u64 {
        return self.data.len();
    }

    fn read(&self) -> Option<u8> {
        return self.data.read();
    }
}
// <- REQUEST

// RESPONSE ->
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

    pub fn finish(&self) -> () {
        let router = self.router.clone();
        let blocking_task = std::thread::spawn(async move || {
            let _ = router.shutdown().await;
        });
        // TODO: HANDLE FAILED JOIN
        let _ = blocking_task.join();
        return ();
    }

    pub fn get_is_finished(&self) -> bool {
        return self.router.is_shutdown() || self.handler.get_is_finished();
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

// exposing them to UniFFI parser
pub use handler::send_files::SendFilesConnectingEvent;
pub use handler::send_files::SendFilesProfile;
pub use handler::send_files::SendFilesSendingEvent;
pub use handler::send_files::SendFilesSubscriber;
// <- RESPONSE

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
    let handler = Arc::new(SendFilesHandler::new(SendFilesHandlerResources {
        profile: Profile {
            id: Uuid::new_v4().to_string(),
            name: request.profile.name,
        },
        files: request
            .files
            .into_iter()
            .map(|f| File {
                id: Uuid::new_v4().to_string(),
                name: f.name,
                data: Arc::new(SenderFileDataAdapter { data: f.data }),
            })
            .collect(),
    }));
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

uniffi::include_scaffolding!("sender");
