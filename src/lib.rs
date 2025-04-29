// TODO: REVIEW WHICH STRUCT/TRAIT/WHATEVER SHOULD BE PUBLIC OR PRIVATE

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use iroh::{
    protocol::{ProtocolHandler, Router},
    Endpoint,
};

use anyhow::Result;
use iroh_base::ticket::NodeTicket;
use n0_future::task::AbortOnDropHandle;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

mod entities;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not create endpoint: \"{0}\".")]
    CreateEndpointError(String),
    #[error("Could not read own endpoint address: \"{0}\".")]
    ReadOwnEndpointAddressError(String),
    #[error("Could not create a router for send files: \"{0}\".")]
    CreateSendFilesRouterError(String),
    #[error("Could not parse the ticket to receive files.")]
    ParseReceiveFilesTicketError,
    #[error("Could not create a connection to receive files: \"{0}\".")]
    CreateReceiveFilesConnectionError(String),
    #[error("Could not create a bidirectional communication with peer: \"{0}\".")]
    CreateBidirectionalCommunicationError(String),
    #[error("Could not activate the bidirectional communication with peer: \"{0}\".")]
    ActivateBidirectionalCommunicationError(String),
    #[error("Could not finish communication input to peer: \"{0}\".")]
    FinishCommunicationInputError(String),
    #[error("Could not finish communication output from peer: \"{0}\".")]
    FinishCommunicationOutputError(String),
    #[error("Could not read communication output from peer: \"{0}\".")]
    ReadCommunicationOutputError(String),
    #[error("Could not deserialize file projection: \"{0}\".")]
    DeserializeFileProjectionError(String),
    #[error("Could not establish a handshake with peer: \"{0}\".")]
    InvalidHandshakeError(String),
    #[error("Could not parse handshake with peer: \"{0}\".")]
    ParseHandshakeError(String),
}

#[derive(Debug, Clone)]
pub struct SendFilesRequest {
    pub files: Vec<FilePayload>,
}

#[derive(Debug, Clone)]
pub struct FilePayload {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SendFilesResponse {
    pub ticket: String,
    pub confirmation: u8,
}

#[derive(Debug, Clone)]
pub struct ReceiveFilesRequest {
    pub ticket: String,
    pub confirmation: u8,
}

#[derive(Debug, Clone)]
pub struct ReceiveFilesResponse {
    pub files: Vec<FileOutput>,
}

#[derive(Debug, Clone)]
pub struct FileOutput {
    pub id: String,
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileDetails {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileProjection {
    pub id: String,
    // TODO: MAYBE WRAP ALL DATA RELATED PROPS TO A STRUCT
    pub total: u64,
    pub offset: u64,
    pub data: Vec<u8>,
    // ---
    pub file: FileDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SendFilesHandshake {
    sender: Profile,
    file_metadatas: Vec<FileMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Profile {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMetadata {
    id: String,
    name: String,
    len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileProjection2 {
    id: String,
    data: Vec<u8>,
}

#[derive(Debug)]
struct SendFilesHandler {
    expires_at: Instant,
    request: SendFilesRequest,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    has_been_consumed: AtomicBool,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    has_finished: Arc<AtomicBool>,
    subscribers: Vec<Arc<dyn SendFilesSubscriber>>,
}

impl SendFilesHandler {
    pub fn new(
        expires_at: Instant,
        request: SendFilesRequest,
        subscribers: Vec<Arc<dyn SendFilesSubscriber>>,
    ) -> Self {
        return Self {
            expires_at,
            request,
            has_been_consumed: AtomicBool::new(false),
            has_finished: Arc::new(AtomicBool::new(false)),
            subscribers,
        };
    }
}

impl ProtocolHandler for SendFilesHandler {
    fn on_connecting(
        &self,
        connecting: iroh::endpoint::Connecting,
    ) -> Pin<Box<dyn Future<Output = Result<iroh::endpoint::Connection>> + Send + 'static>> {
        if Instant::now() > self.expires_at {
            return Box::pin(async {
                return Err(anyhow::anyhow!("Connection handler has expired."));
            });
        }
        let has_been_consumed = self
            .has_been_consumed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .unwrap_or(true);
        if has_been_consumed {
            return Box::pin(async {
                return Err(anyhow::anyhow!("Connection has already been consumed."));
            });
        }
        return Box::pin(async { Ok(connecting.await?) });
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        return Box::pin(async {});
    }

    fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        let has_finished = self.has_finished.clone();
        let request = self.request.clone();
        // let subscribers = self.subscribers.clone();
        return Box::pin(async move {
            let mut bi = connection.accept_bi().await?;
            let identified_files: HashMap<String, &FilePayload> = HashMap::from_iter(
                request
                    .files
                    .iter()
                    .map(|f| return (Uuid::new_v4().to_string(), f)),
            );
            let my_handshake = SendFilesHandshake {
                sender: Profile {
                    name: String::from("TODO: PROFILE SYSTEM"),
                },
                file_metadatas: identified_files
                    .iter()
                    .map(|(id, f)| {
                        return FileMetadata {
                            id: id.clone(),
                            len: f.data.len() as u64,
                            name: f.name.clone(),
                        };
                    })
                    .collect(),
            };
            let my_handshake_serialized = serde_json::to_vec(&my_handshake).unwrap();
            // TODO: LIMIT HOW MUCH FILES IT IS POSSIBLE TO SEND
            let my_handshake_serialized_len = my_handshake_serialized.len() as u32;
            let my_handshake_serialized_len_bytes = my_handshake_serialized_len.to_be_bytes();
            // NOTIFY RECEIVER ABOUT THE HANDSHAKE LENGTH
            bi.0.write_all(&my_handshake_serialized_len_bytes).await?;
            // SEND HANDSHAKE
            bi.0.write_all(&my_handshake_serialized).await?;

            for (id, f) in identified_files {
                let chunk_len = 1024; // TODO: FLEXIBILIZE CHUNK LEN

                for chunk in f.data.chunks(chunk_len) {
                    let projection = FileProjection2 {
                        id: id.clone(),
                        data: chunk.to_vec(),
                    };
                    let serialized_projection = serde_json::to_vec(&projection).unwrap();
                    // TODO: LIMIT CHUNK LEN **NEAR** U16, BECAUSE JSON TAKES MORE SPACE
                    let serialized_projection_len = serialized_projection.len() as u16;
                    let serialized_projection_len_bytes = serialized_projection_len.to_be_bytes();
                    // NOTIFY RECEIVER ABOUT THE HANDSHAKE LENGTH
                    bi.0.write_all(&serialized_projection_len_bytes).await?;
                    // SEND PROJECTION
                    bi.0.write_all(&serialized_projection).await?;
                }
            }

            bi.0.finish()?;
            bi.0.stopped().await?;
            connection.close(iroh::endpoint::VarInt::from_u32(0), &[0]);
            has_finished.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        });
    }
}

pub struct SendingEvent {
    // TODO
}

pub struct ReceivingEvent {
    // TODO
}

pub struct ConnectingEvent {
    // TODO
}

pub trait SendFilesSubscriber: Send + Sync + std::fmt::Debug {
    fn notify_sending(&self, event: SendingEvent);
    fn notify_connecting(&self, event: ConnectingEvent);
}

pub trait ReceiveFilesSubscriber: Send + Sync + std::fmt::Debug {
    fn notify_receiving(&self, event: ReceivingEvent);
    fn notify_connecting(&self, event: ConnectingEvent);
}

struct SendFilesConfiguration {
    router: Arc<Router>,
    handler: Arc<SendFilesHandler>,
}

pub struct Drop {
    _control_task: AbortOnDropHandle<()>,
    send_files_subscribers: RwLock<Vec<Arc<dyn SendFilesSubscriber>>>,
    receive_files_subscribers: RwLock<Vec<Arc<dyn ReceiveFilesSubscriber>>>,
    send_files_configurations: Arc<Mutex<Vec<SendFilesConfiguration>>>,
}

impl Drop {
    pub fn new() -> Result<Self, Error> {
        let send_files_subscribers = RwLock::new(Vec::new());
        let receive_files_subscribers = RwLock::new(Vec::new());
        let send_files_configurations = Arc::new(Mutex::new(Vec::<SendFilesConfiguration>::new()));

        let c_send_files_configurations = send_files_configurations.clone();
        let _control_task = AbortOnDropHandle::new(tokio::task::spawn(async move {
            loop {
                let mut send_files_configurations = c_send_files_configurations.lock().await;
                loop {
                    let now = Instant::now();
                    let maybe_idx = send_files_configurations.iter().position(|c| {
                        return c.router.is_shutdown()
                            || c.handler
                                .has_finished
                                .load(std::sync::atomic::Ordering::Relaxed)
                            || now > c.handler.expires_at;
                    });
                    if maybe_idx.is_none() {
                        break;
                    }
                    let config_idx = maybe_idx.unwrap();
                    let config = send_files_configurations.remove(config_idx);
                    let _ = config.router.shutdown().await;
                }
            }
        }));

        return Ok(Self {
            _control_task,
            send_files_subscribers,
            receive_files_subscribers,
            send_files_configurations,
        });
    }

    pub async fn subscribe_to_send_files(&self, subscriber: Arc<dyn SendFilesSubscriber>) -> () {
        self.send_files_subscribers.write().await.push(subscriber);
    }

    pub async fn subscribe_to_receive_files(
        &self,
        subscriber: Arc<dyn ReceiveFilesSubscriber>,
    ) -> () {
        self.receive_files_subscribers
            .write()
            .await
            .push(subscriber);
    }

    pub async fn send_files(&self, request: SendFilesRequest) -> Result<SendFilesResponse, Error> {
        /*
           TODO:
               - Limit how many send files configs is possible to exist at the same time
               - Maybe let the API client decide the expiration time
        */
        let endpoint = Endpoint::builder()
            .discovery_n0()
            .bind()
            .await
            .map_err(|e| Error::CreateEndpointError(e.to_string()))?;
        let node_addr = endpoint
            .node_addr()
            .await
            .map_err(|e| Error::ReadOwnEndpointAddressError(e.to_string()))?;

        let confirmation: u8 = rand::rng().random_range(0..=99);
        let handler = Arc::new(SendFilesHandler::new(
            Instant::now() + Duration::from_secs(60 * 15),
            request.clone(),
            self.send_files_subscribers.read().await.clone(),
        ));
        let router = Arc::new(
            Router::builder(endpoint)
                .accept([confirmation], handler.clone())
                .spawn()
                .await
                .map_err(|e| return Error::CreateSendFilesRouterError(e.to_string()))?,
        );
        self.send_files_configurations
            .lock()
            .await
            .push(SendFilesConfiguration { router, handler });

        return Ok(SendFilesResponse {
            ticket: NodeTicket::new(node_addr).to_string(),
            confirmation,
        });
    }

    pub async fn receive_files(
        &self,
        request: ReceiveFilesRequest,
    ) -> Result<ReceiveFilesResponse, Error> {
        /*
           TODO:
               - Should not allow receiving files from itself
        */
        let ticket: NodeTicket = request
            .ticket
            .parse()
            .map_err(|_| Error::ParseReceiveFilesTicketError)?;

        let endpoint = Endpoint::builder()
            .discovery_n0()
            .bind()
            .await
            .map_err(|e| Error::CreateEndpointError(e.to_string()))?;

        let connection = endpoint
            .connect(ticket, &[request.confirmation])
            .await
            .map_err(|e| Error::CreateReceiveFilesConnectionError(e.to_string()))?;

        let mut bi = connection
            .open_bi()
            .await
            .map_err(|e| Error::CreateBidirectionalCommunicationError(e.to_string()))?;

        // TODO: SEND MY HANDSHAKE
        bi.0.write_all(&[0])
            .await
            .map_err(|e| Error::ActivateBidirectionalCommunicationError(e.to_string()))?;
        bi.0.finish()
            .map_err(|e| Error::FinishCommunicationInputError(e.to_string()))?;

        let maybe_their_handshake_len_raw =
            bi.1.read_chunk(4, true)
                .await
                .map_err(|e| Error::ReadCommunicationOutputError(e.to_string()))?;

        if maybe_their_handshake_len_raw.is_none() {
            bi.1.stop(iroh::endpoint::VarInt::from_u32(0))
                .map_err(|e| Error::FinishCommunicationOutputError(e.to_string()))?;
            endpoint.close().await;
            return Err(Error::InvalidHandshakeError(String::from(
                "Handshake bytes length not found",
            )));
        }

        let their_handshake_len_raw = maybe_their_handshake_len_raw.unwrap();
        let their_handshake_len_bytes: [u8; 4] =
            their_handshake_len_raw.bytes[0..4].try_into().unwrap();
        let their_handshake_len: u32 = u32::from_be_bytes(their_handshake_len_bytes);

        let maybe_their_handshake_raw =
            bi.1.read_chunk(their_handshake_len as usize, true)
                .await
                .map_err(|e| Error::ReadCommunicationOutputError(e.to_string()))?;

        if maybe_their_handshake_raw.is_none() {
            bi.1.stop(iroh::endpoint::VarInt::from_u32(0))
                .map_err(|e| Error::FinishCommunicationOutputError(e.to_string()))?;
            endpoint.close().await;
            return Err(Error::InvalidHandshakeError(String::from(
                "Handshake not found",
            )));
        }

        let their_handshake_raw = maybe_their_handshake_raw.unwrap();
        let their_handshake: SendFilesHandshake =
            serde_json::from_slice(&their_handshake_raw.bytes)
                .map_err(|e| Error::ParseHandshakeError(e.to_string()))?;

        let mut files: Vec<FileOutput> = Vec::new();
        loop {
            let maybe_chunk_len_raw =
                bi.1.read_chunk(2, true)
                    .await
                    .map_err(|e| Error::ReadCommunicationOutputError(e.to_string()))?;
            if maybe_chunk_len_raw.is_none() {
                break;
            }
            let chunk_len_raw = maybe_chunk_len_raw.unwrap();
            let chunk_len_bytes: [u8; 2] = chunk_len_raw.bytes[0..2].try_into().unwrap();
            let chunk_len: u16 = u16::from_be_bytes(chunk_len_bytes);

            let maybe_chunk_raw =
                bi.1.read_chunk(chunk_len as usize, true)
                    .await
                    .map_err(|e| Error::ReadCommunicationOutputError(e.to_string()))?;
            if maybe_chunk_raw.is_none() {
                break;
            }
            let chunk_raw = maybe_chunk_raw.unwrap();
            let projection: FileProjection2 = serde_json::from_slice(&chunk_raw.bytes)
                .map_err(|e| Error::DeserializeFileProjectionError(e.to_string()))?;

            let maybe_file = files.iter_mut().find(|f| f.id == projection.id);
            if maybe_file.is_some() {
                let file = maybe_file.unwrap();
                file.data.append(&mut projection.data.clone());
            } else {
                // TODO: HANDLE FILE METADATA NOT FOUND
                // TODO: HANDLE (MAYBE NOT HERE) FILE DATA LEN GREATER THAN EXPECTED ON METADATA
                let file_metadata = their_handshake
                    .file_metadatas
                    .iter()
                    .find(|f| f.id == projection.id)
                    .unwrap();
                let file = FileOutput {
                    id: file_metadata.id.clone(),
                    name: file_metadata.name.clone(),
                    data: projection.data.clone(),
                };
                files.push(file);
            }
        }

        endpoint.close().await;
        return Ok(ReceiveFilesResponse { files });
    }
}

uniffi::include_scaffolding!("drop");
