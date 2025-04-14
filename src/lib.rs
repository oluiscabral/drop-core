use iroh::{protocol::Router, Endpoint};
use iroh_blobs::{
    net_protocol::Blobs, rpc::client::blobs::AddOutcome, store::mem::Store, ticket::BlobTicket,
    util::SetTagOption, BlobFormat,
};

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

#[derive(Debug)]
pub struct SendFilesRequest {
    pub files: Vec<FilePayload>,
}

#[derive(Debug)]
pub struct FilePayload {
    pub name: String,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct SendFilesResponse {
    pub ticket: TicketOutput,
}

#[derive(Debug)]
pub struct TicketOutput {
    pub id: String,
}

#[derive(Debug)]
pub struct ReceiveFilesRequest {
    pub ticket: TicketPayload,
}

#[derive(Debug)]
pub struct TicketPayload {
    pub id: String,
}

#[derive(Debug)]
pub struct ReceiveFilesResponse {
    pub bag: BagOutput,
}

#[derive(Debug)]
pub struct BagOutput {
    pub ticket: TicketOutput,
    pub files: Vec<FileOutput>,
}

#[derive(Debug)]
pub struct FileOutput {
    pub id: String,
    pub name: String,
    pub data: Vec<u8>,
}

pub struct Drop {
    blobs: Blobs<Store>,
    router: Router,
}

impl Drop {
    pub async fn new() -> Result<Self, Error> {
        let endpoint = Endpoint::builder()
            .discovery_n0()
            .bind()
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::InitializeEnvironmentError;
            })?;
        let blobs = Blobs::memory().build(&endpoint);
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, blobs.clone())
            .spawn()
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::InitializeEnvironmentError;
            })?;
        return Ok(Self { blobs, router });
    }

    async fn send_files(&self, request: SendFilesRequest) -> Result<SendFilesResponse, Error> {
        let node_id = self.router.endpoint().node_id();

        let mut outcomes: Vec<AddOutcome> = vec![];
        for file in &request.files {
            let outcome = self
                .blobs
                .client()
                .add_bytes(file.data.clone())
                .await
                .map_err(|e| {
                    eprintln!("{}", e.to_string());
                    return Error::ReadFileDataError(file.name.clone());
                })?;
            outcomes.push(outcome);
        }

        let collection = outcomes
            .iter()
            .zip(request.files.iter())
            .map(|(outcome, file)| (file.name.clone(), outcome.hash))
            .collect();
        let collection_outcome = self
            .blobs
            .client()
            .create_collection(collection, SetTagOption::Auto, Default::default())
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::CreateCollectionError;
            })?;

        let ticket = BlobTicket::new(node_id.into(), collection_outcome.0, BlobFormat::HashSeq)
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::CreateTicketError;
            })?;

        return Ok(SendFilesResponse {
            ticket: TicketOutput {
                id: ticket.to_string(),
            },
        });
    }

    async fn receive_files(
        &self,
        request: ReceiveFilesRequest,
    ) -> Result<ReceiveFilesResponse, Error> {
        let ticket: BlobTicket = request.ticket.id.parse().map_err(|_| {
            return Error::ParseTicketError;
        })?;
        if ticket.format() != BlobFormat::HashSeq {
            return Err(Error::InvalidTicketFormatError);
        }

        self.blobs
            .client()
            .download_hash_seq(ticket.hash(), ticket.node_addr().clone())
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::InitializeDownloadError;
            })?
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::FinishDownloadError;
            })?;

        let collection = self
            .blobs
            .client()
            .get_collection(ticket.hash())
            .await
            .map_err(|e| {
                eprintln!("{}", e.to_string());
                return Error::RetrieveCollectionError;
            })?;

        let mut files: Vec<FileOutput> = vec![];
        for (name, hash) in collection.iter() {
            let data = self
                .blobs
                .client()
                .read_to_bytes(hash.clone())
                .await
                .map_err(|e| {
                    eprintln!("{}", e.to_string());
                    return Error::ReadFileDataError(name.clone());
                })?;
            let file = FileOutput {
                id: hash.to_string(),
                name: name.clone(),
                data: data.to_vec(),
            };
            files.push(file);
        }

        return Ok(ReceiveFilesResponse {
            bag: BagOutput {
                files,
                ticket: TicketOutput {
                    id: request.ticket.id,
                },
            },
        });
    }
}

uniffi::include_scaffolding!("drop");
