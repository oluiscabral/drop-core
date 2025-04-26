// TODO: REVIEW WHICH STRUCT/TRAIT/WHATEVER SHOULD BE PUBLIC OR PRIVATE

use std::{
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
    #[error("Could not initialize the data exchange environment.")]
    InitializeEnvironmentError,
    #[error("Could not read file `{0}` data.")]
    ReadFileDataError(String),
    #[error("Could not create a collection in the data exchange environment.")]
    CreateCollectionError,
    #[error("Could not create the P2P authentication ticket.")]
    CreateTicketError,
    #[error("Could not parse the P2P authentication ticket.")]
    ParseTicketError,
    #[error("P2P authentication ticket format is invalid.")]
    InvalidTicketFormatError,
    #[error("Could not initialize downloading data from the data exchange environment.")]
    InitializeDownloadError,
    #[error("Could not finish downloading data from the data exchange environment.")]
    FinishDownloadError,
    #[error("Could not retrieve file collection from the data exchange environment.")]
    RetrieveCollectionError,
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
        self.subscribers.iter().for_each(|s| {
            s.notify(SendFilesEvent::Connecting {
                // TODO
            });
        });
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
        self.subscribers.iter().for_each(|s| {
            s.notify(SendFilesEvent::Closed {
                // TODO
            });
        });
        return Box::pin(async {});
    }

    fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        let has_finished = self.has_finished.clone();
        let request = self.request.clone();
        let subscribers = self.subscribers.clone();
        return Box::pin(async move {
            println!("EXECUTING THE CONNECTION ACCEPTED HANDLER");
            // TODO: ERROR HANDLING
            let mut bi = connection.accept_bi().await?;
            println!("ACCEPTED BI!");
            subscribers.iter().for_each(|subscriber| {
                subscriber.notify(SendFilesEvent::Connected {
                                // TODO
                            });
            });
            bi.1.read(&mut vec![]).await?;
            println!("READ 'HANDSHAKE'");
            // TODO: RECEIVES THEIR PROFILE AND NOTIFY SUBSCRIBERS
            // TODO: SENDS MY PROFILE
            for f in &request.files {
                // TODO: SENDS FILE IN SMALL PACKETS, WITH A HEADER CONTAINING THEIR METADATA
                let id = Uuid::new_v4().to_string();
                let data: Vec<u8> = f.data.clone();
                let total: u64 = data.len().try_into().unwrap();
                let chunk_len = 1024; // TODO: flexibilize chunk size

                let mut offset = 0;
                for chunk in data.chunks(chunk_len) {
                    let len = chunk.len() as u64;
                    let end = offset + len;

                    let projection = FileProjection {
                        id: id.clone(),
                        total,
                        offset,
                        data: chunk.to_vec(),
                        file: FileDetails {
                            name: f.name.clone(),
                        },
                    };
                    let serialized_projection = serde_json::to_vec(&projection).unwrap();
                    let serialized_projection_bytes =
                        uniffi::deps::bytes::Bytes::copy_from_slice(&serialized_projection);

                    subscribers.iter().for_each(|subscriber| {
                        subscriber.notify(SendFilesEvent::Sending {
                                        // TODO
                                    });
                    });

                    // TODO: ERROR HANDLING
                    bi.0.write_chunk(serialized_projection_bytes).await?;

                    subscribers.iter().for_each(|subscriber| {
                        subscriber.notify(SendFilesEvent::Sent {
                                    // TODO
                                });
                    });

                    offset = end;
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

pub enum SendFilesEvent {
    Connecting {
        // TODO
    },
    Connected {
        // TODO
    },
    Sending {
        // TODO
    },
    Sent {
        // TODO
    },
    Closed {
        // TODO
    },
}

impl std::fmt::Display for SendFilesEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendFilesEvent::Connecting {
                // TODO
            } => f.write_str("connecting"),
            SendFilesEvent::Connected {
                // TODO
            } => f.write_str("connected"),
            SendFilesEvent::Sending {
                // TODO
            } => f.write_str("sending"),
            SendFilesEvent::Sent {
                // TODO
            } => f.write_str("sent"),
            SendFilesEvent::Closed {
                // TODO
            } => f.write_str("closed"),
        }
    }
}

pub enum ReceiveFilesEvent {
    RECEIVING {
        // TODO
    },
}

impl std::fmt::Display for ReceiveFilesEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiveFilesEvent::RECEIVING {
                // TODO
            } => f.write_str("receiving"),
        }
    }
}

pub trait SendFilesSubscriber: Send + Sync + std::fmt::Debug {
    fn notify(&self, event: SendFilesEvent) -> ();
}

pub trait ReceiveFilesSubscriber: Send + Sync + std::fmt::Debug {
    fn notify(&self, event: ReceiveFilesEvent) -> ();
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
        let confirmation: u8 = rand::rng().random_range(0..=99);

        let endpoint = Endpoint::builder()
            .discovery_n0()
            .bind()
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                // TODO: REVIEW ERROR NAME
                return Error::InitializeEnvironmentError;
            })?;
        let node_addr = endpoint.node_addr().await.map_err(|e| {
            eprintln!("{}", e.to_string());
            // TODO: REVIEW ERROR NAME
            return Error::InitializeEnvironmentError;
        })?;

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
                .map_err(|e| {
                    eprintln!("{}", e.to_string());
                    return Error::InitializeEnvironmentError;
                })?,
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
        let ticket: NodeTicket = request.ticket.parse().map_err(|_| {
            return Error::ParseTicketError;
        })?;

        let endpoint = Endpoint::builder()
            .discovery_n0()
            .bind()
            .await
            .map_err(|e| {
                eprintln!("endpoint error {}", e.to_string());
                // TODO: REVIEW ERROR NAME
                return Error::InitializeEnvironmentError;
            })?;

        let connection = endpoint
            .connect(ticket, &[request.confirmation])
            .await
            .map_err(|e| {
                eprintln!("connection error {}", e.to_string());
                // TODO: UPDATE ERROR NAME
                return Error::InitializeDownloadError;
            })?;

        let mut bi = connection.open_bi().await.map_err(|e| {
            eprintln!("bi error {}", e.to_string());
            // TODO: UPDATE ERROR NAME
            return Error::InitializeDownloadError;
        })?;

        bi.0.write_all(&[0]).await.map_err(|e| {
            eprintln!("initial bi write error {}", e.to_string());
            // TODO: UPDATE ERROR NAME
            return Error::InitializeDownloadError;
        })?;
        bi.0.finish().map_err(|e| {
            eprintln!("finish bi write error {}", e.to_string());
            // TODO: UPDATE ERROR NAME
            return Error::InitializeDownloadError;
        })?;

        // TODO: SEND MY PROFILE
        // TODO: RECEIVES THEIR PROFILE (AND POSSIBLY CHUNK SIZE??)
        let chunk_len = 1024; // TODO: flexibilize chunk size
        let mut files: Vec<FileOutput> = Vec::new();
        loop {
            let maybe_chunk = bi.1.read_chunk(chunk_len, true).await.map_err(|e| {
                eprintln!("read chunk error {}", e.to_string());
                // TODO: UPDATE ERROR NAME
                return Error::InitializeDownloadError;
            })?;
            if maybe_chunk.is_none() {
                break;
            }
            let chunk = maybe_chunk.unwrap();
            let projection: FileProjection = serde_json::from_slice(&chunk.bytes).map_err(|e| {
                eprintln!("deserializing projection error {}", e.to_string());
                // TODO: UPDATE ERROR NAME
                return Error::InitializeDownloadError;
            })?;
            self.receive_files_subscribers
                .read()
                .await
                .iter()
                .for_each(|s| {
                    s.notify(ReceiveFilesEvent::RECEIVING {
                    // TODO
                });
                });

            let maybe_file = files.iter_mut().find(|f| f.id == projection.id);
            if maybe_file.is_some() {
                let file = maybe_file.unwrap();
                file.data.append(&mut projection.data.clone());
            } else {
                let file = FileOutput {
                    id: projection.id.clone(),
                    name: projection.file.name,
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
