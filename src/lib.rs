// TODO: REVIEW WHICH STRUCT/TRAIT/WHATEVER SHOULD BE PUBLIC OR PRIVATE

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use iroh::{
    protocol::{ProtocolHandler, Router},
    Endpoint,
};

use anyhow::Result;
use iroh_base::ticket::NodeTicket;
use rand::Rng;
use serde::{Deserialize, Serialize};
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
    pub total: u64,
    pub offset: u64,
    pub file: FileDetails,
    pub data: Vec<u8>,
}

#[derive(Debug)]
struct SendFilesHandler {
    request: SendFilesRequest,
    subscribers: Vec<Arc<dyn SendFilesSubscriber>>,
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
        return Box::pin(async {
            let conn = connecting.await?;
            Ok(conn)
        });
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
        let request = self.request.clone();
        let subscribers = self.subscribers.clone();
        return Box::pin(async move {
            // TODO: ERROR HANDLING
            let mut bi = connection.open_bi().await?;
            // TODO: SENDS MY PROFILE
            // TODO: RECEIVES THEIR PROFILE AND NOTIFY SUBSCRIBERS
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
                    // TODO: ERROR HANDLING
                    bi.0.write_chunk(serialized_projection_bytes).await?;

                    offset = end;
                }
            }
            // TODO: CLOSE THE CONNECTION
            return Ok(());
        });
    }
}

struct SendFilesConfiguration {
    pub router: Router,
    pub expires_at: Instant,
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

pub trait SendFilesSubscriber: Send + Sync + std::fmt::Debug {
    fn notify(&self, event: SendFilesEvent) -> ();
}

struct SendFilesResources {
    subscribers: Vec<Arc<dyn SendFilesSubscriber>>,
    configurations: Vec<SendFilesConfiguration>,
}

pub struct Drop {
    send_files_resources: RwLock<SendFilesResources>,
}

impl Drop {
    pub fn new() -> Result<Self, Error> {
        return Ok(Self {
            send_files_resources: RwLock::new(SendFilesResources {
                subscribers: Vec::new(),
                configurations: Vec::new(),
            }),
        });
    }

    fn subscribe_to_send_files(&self, subscriber: Arc<dyn SendFilesSubscriber>) -> () {
        self.send_files_resources
            .write()
            .unwrap()
            .subscribers
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

        let handler = SendFilesHandler {
            request: request.clone(),
            subscribers: self
                .send_files_resources
                .read()
                .unwrap()
                .subscribers
                .clone(),
        };
        let router = Router::builder(endpoint)
            .accept([confirmation], handler)
            .spawn()
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::InitializeEnvironmentError;
            })?;
        let config = SendFilesConfiguration {
            router,
            expires_at: Instant::now() + Duration::from_secs(60 * 15),
        };
        self.send_files_resources
            .write()
            .unwrap()
            .configurations
            .push(config);

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
                eprintln!("{}", e.to_string());
                // TODO: REVIEW ERROR NAME
                return Error::InitializeEnvironmentError;
            })?;

        let connection = endpoint
            .connect(ticket, &[request.confirmation])
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                // TODO: UPDATE ERROR NAME
                return Error::InitializeDownloadError;
            })?;

        // TODO: PROPERLY RECEIVE FILES
        println!(
            "RECEIVING FILES FROM '{}'",
            connection.remote_node_id().unwrap()
        );

        // TODO
        return Ok(ReceiveFilesResponse { files: Vec::new() });
    }
}

uniffi::include_scaffolding!("drop");
