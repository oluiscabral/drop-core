use std::{
    future::Future,
    pin::Pin,
    sync::RwLock,
    time::{Duration, Instant},
};

use iroh::{
    protocol::{ProtocolHandler, Router},
    Endpoint,
};

use anyhow::Result;
use iroh_base::ticket::NodeTicket;
use rand::Rng;

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

#[derive(Debug)]
struct SendFilesHandler {
    request: SendFilesRequest,
}

impl ProtocolHandler for SendFilesHandler {
    fn on_connecting(
        &self,
        connecting: iroh::endpoint::Connecting,
    ) -> Pin<Box<dyn Future<Output = Result<iroh::endpoint::Connection>> + Send + 'static>> {
        return Box::pin(async move {
            let conn = connecting.await?;
            Ok(conn)
        });
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        return Box::pin(async {});
    }

    fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        println!(
            "SENDING FILES TO '{}'",
            connection.remote_node_id().unwrap()
        );
        return Box::pin(async { Ok(()) });
    }
}

struct SendFilesConfiguration {
    pub router: Router,
    pub expires_at: Instant,
}

pub struct Drop {
    send_files_configurations: RwLock<Vec<SendFilesConfiguration>>,
}

impl Drop {
    pub fn new() -> Result<Self, Error> {
        return Ok(Self {
            send_files_configurations: RwLock::new(Vec::new()),
        });
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
        self.send_files_configurations.write().unwrap().push(config);

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

        println!(
            "RECEIVING FILES FROM '{}'",
            connection.remote_node_id().unwrap()
        );

        // TODO
        return Ok(ReceiveFilesResponse { files: Vec::new() });
    }
}

uniffi::include_scaffolding!("drop");
