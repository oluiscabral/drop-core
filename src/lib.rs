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

static mut BLOBS: Option<Blobs<Store>> = None;
static mut ROUTER: Option<Router> = None;

async fn ensure_iroh() -> Result<(&'static Blobs<Store>, &'static Router), Error> {
    if is_iroh_uninitialized() {
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
        unsafe {
            BLOBS = Some(blobs);
            ROUTER = Some(router);
        };
    }
    return Ok(unsafe { (BLOBS.as_ref().unwrap(), ROUTER.as_ref().unwrap()) });
}

fn is_iroh_uninitialized() -> bool {
    return unsafe { BLOBS.is_none() || ROUTER.is_none() };
}

pub async fn send_files(request: SendFilesRequest) -> Result<SendFilesResponse, Error> {
    let (blobs, router) = ensure_iroh().await?;
    let node_id = router.endpoint().node_id();

    let mut outcomes: Vec<AddOutcome> = vec![];
    for file in &request.files {
        let outcome = blobs
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
    let collection_outcome = blobs
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

pub async fn receive_files(request: ReceiveFilesRequest) -> Result<ReceiveFilesResponse, Error> {
    let (blobs, _) = ensure_iroh().await?;

    let ticket: BlobTicket = request.ticket.id.parse().map_err(|_| {
        return Error::ParseTicketError;
    })?;
    if ticket.format() != BlobFormat::HashSeq {
        return Err(Error::InvalidTicketFormatError);
    }

    blobs
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

    let collection = blobs
        .client()
        .get_collection(ticket.hash())
        .await
        .map_err(|e| {
            eprintln!("{}", e.to_string());
            return Error::RetrieveCollectionError;
        })?;

    let mut files: Vec<FileOutput> = vec![];
    for (name, hash) in collection.iter() {
        let data = blobs
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

#[cfg(test)]
mod test {
    use std::{fs, path::PathBuf};

    use crate::{
        receive_files, send_files, FilePayload, ReceiveFilesRequest, SendFilesRequest,
        TicketPayload, BLOBS, ROUTER,
    };

    #[tokio::test]
    async fn test_send_files() {
        // Setup
        let file_path1 = PathBuf::from("./test_file1.txt");
        let file_path2 = PathBuf::from("./test_file2.txt");
        let file_path3 = PathBuf::from("./test_file3.txt");
        std::fs::write(&file_path1, "content1").unwrap();
        std::fs::write(&file_path2, "content2").unwrap();
        std::fs::write(&file_path3, "content3").unwrap();
        let file_paths = vec![
            fs::canonicalize(&file_path1).unwrap(),
            fs::canonicalize(&file_path2).unwrap(),
            fs::canonicalize(&file_path3).unwrap(),
        ];
        let files = file_paths
            .iter()
            .map(|path| {
                let content = std::fs::read(path).unwrap();
                return FilePayload {
                    name: path.file_name().unwrap().to_str().unwrap().to_string(),
                    data: content.to_vec(),
                };
            })
            .collect();

        // Run
        let request = SendFilesRequest { files };
        let result = send_files(request).await;

        // Assert
        unsafe {
            assert!(BLOBS.is_some());
            assert!(ROUTER.is_some());
        }
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.ticket.id.is_empty(), "Ticket should not be empty");

        // Clean up
        std::fs::remove_file(&file_path1).unwrap();
        std::fs::remove_file(&file_path2).unwrap();
        std::fs::remove_file(&file_path3).unwrap();
    }

    #[tokio::test]
    async fn test_receive_files() {
        // Setup
        let file_path1 = PathBuf::from("./test_file1.txt");
        let file_path2 = PathBuf::from("./test_file2.txt");
        let file_path3 = PathBuf::from("./test_file3.txt");
        std::fs::write(&file_path1, "content1").unwrap();
        std::fs::write(&file_path2, "content2").unwrap();
        std::fs::write(&file_path3, "content3").unwrap();
        let file_paths = vec![
            fs::canonicalize(&file_path1).unwrap(),
            fs::canonicalize(&file_path2).unwrap(),
            fs::canonicalize(&file_path3).unwrap(),
        ];
        let mut payload_files = file_paths.iter().map(|path| {
            let content = std::fs::read(path).unwrap();
            return FilePayload {
                name: path.file_name().unwrap().to_str().unwrap().to_string(),
                data: content.to_vec(),
            };
        });
        let send_request = SendFilesRequest {
            files: payload_files.clone().collect(),
        };
        let send_response = send_files(send_request).await.unwrap();

        // Run
        let request = ReceiveFilesRequest {
            ticket: TicketPayload {
                id: send_response.ticket.id.clone(),
            },
        };
        let result = receive_files(request).await;

        // Assert
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.bag.ticket.id, send_response.ticket.id);
        assert_eq!(response.bag.files.len(), 3);
        for file in &response.bag.files {
            assert!(!file.id.is_empty(), "Ticket should not be empty");
            let maybe_payload_file = payload_files.find(|pf| pf.name == file.name);
            assert!(maybe_payload_file.is_some());
            let payload_file = maybe_payload_file.unwrap();
            assert_eq!(file.data, payload_file.data);
        }

        // Clean up
        std::fs::remove_file(&file_path1).unwrap();
        std::fs::remove_file(&file_path2).unwrap();
        std::fs::remove_file(&file_path3).unwrap();
    }
}

uniffi::include_scaffolding!("drop");
