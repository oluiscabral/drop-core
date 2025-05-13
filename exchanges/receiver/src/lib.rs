// TODO: DOUBLE CHECK ALL `unwrap`
// TODO: move interactors to different modules

use anyhow::Result;
use common::{FileProjection, HandshakeProfile, ReceiverHandshake, SenderHandshake};
use entities::Profile;
use iroh::{
    Endpoint,
    endpoint::{RecvStream, SendStream},
};
use iroh_base::ticket::NodeTicket;
use std::{
    io::{Read, Write},
    os::unix::fs::FileExt,
    sync::{Arc, atomic::AtomicU64},
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ReceiverError {
    #[error("TODO: \"{0}\".")]
    TODO(String),
}

// REQUEST ->
pub struct ReceiveFilesRequest {
    pub ticket: String,
    pub confirmation: u8,
    pub profile: ReceiverProfile,
}

pub struct ReceiverProfile {
    pub name: String,
}
// <- REQUEST

// RESPONSE ->
#[derive(Debug)]
pub struct ReceiveFilesResponse {
    pub files: Vec<ReceiverFile>,
}

#[derive(Debug)]
pub struct ReceiverFile {
    pub id: String,
    pub name: String,
    pub data: Arc<ReceiverFileData>,
}

#[derive(Debug)]
pub struct ReceiverFileData {
    offset: AtomicU64,
    path: std::path::PathBuf,
}
impl ReceiverFileData {
    pub fn new(path: std::path::PathBuf) -> Self {
        return Self {
            offset: AtomicU64::new(0),
            path,
        };
    }

    pub fn len(&self) -> u64 {
        let file = std::fs::File::open(&self.path).unwrap();
        return file.bytes().count() as u64;
    }

    pub fn read(&self) -> Option<u8> {
        let offset = self.offset.load(std::sync::atomic::Ordering::Acquire);
        let mut buf = [0u8; 1];
        let file = std::fs::File::open(&self.path).unwrap();
        let read_result = file.read_exact_at(buf.as_mut_slice(), offset);
        if read_result.is_err() {
            return None;
        }
        self.offset
            .store(offset + 1, std::sync::atomic::Ordering::Release);
        return Some(buf[0]);
    }
}
// <- RESPONSE

struct ReceiveFilesResources {
    pub profile: Profile,
}

pub async fn receive_files(
    request: ReceiveFilesRequest,
) -> Result<ReceiveFilesResponse, ReceiverError> {
    /*
       TODO:
           - User should not receive files from themself
    */
    let ticket: NodeTicket = request
        .ticket
        .parse()
        .map_err(|_| ReceiverError::TODO("".to_string()))?;

    let endpoint = Endpoint::builder()
        .discovery_n0()
        .bind()
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;
    let connection = endpoint
        .connect(ticket, &[request.confirmation])
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;
    let (mut stream, mut their_stream) = connection
        .open_bi()
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;

    let resources = create_resources(request);
    send_handshake(&mut stream, &resources)
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;
    stream
        .finish()
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;

    let their_handshake = get_their_handshake(&mut their_stream)
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;
    if their_handshake.is_none() {
        return Err(ReceiverError::TODO("".to_string()));
    }
    let their_handshake = their_handshake.unwrap();
    let files = receive_stream_files(&mut their_stream, &their_handshake)
        .await
        .map_err(|e| ReceiverError::TODO(e.to_string()))?;

    endpoint.close().await;
    return Ok(ReceiveFilesResponse { files });
}

fn create_resources(request: ReceiveFilesRequest) -> ReceiveFilesResources {
    return ReceiveFilesResources {
        profile: Profile {
            id: Uuid::new_v4().to_string(),
            name: request.profile.name,
        },
    };
}

async fn send_handshake(stream: &mut SendStream, resources: &ReceiveFilesResources) -> Result<()> {
    let handshake = ReceiverHandshake {
        profile: HandshakeProfile {
            id: resources.profile.id.clone(),
            name: resources.profile.name.clone(),
        },
    };
    let handshake_serialized = serde_json::to_vec(&handshake).unwrap();
    let handshake_serialized_len = handshake_serialized.len() as u32;
    let handshake_serialized_len_bytes = handshake_serialized_len.to_be_bytes();
    stream.write_all(&handshake_serialized_len_bytes).await?;
    stream.write_all(&handshake_serialized).await?;
    return Ok(());
}

async fn get_their_handshake(their_stream: &mut RecvStream) -> Result<Option<SenderHandshake>> {
    let their_handshake_len_raw = their_stream.read_chunk(4, true).await?;
    if their_handshake_len_raw.is_none() {
        return Ok(None);
    }
    let their_handshake_len_raw = their_handshake_len_raw.unwrap();
    let their_handshake_len_bytes: [u8; 4] =
        their_handshake_len_raw.bytes[0..4].try_into().unwrap();
    let their_handshake_len: u32 = u32::from_be_bytes(their_handshake_len_bytes);
    let their_handshake_raw = their_stream
        .read_chunk(their_handshake_len as usize, true)
        .await?;
    if their_handshake_raw.is_none() {
        return Ok(None);
    }
    let their_handshake_raw = their_handshake_raw.unwrap();
    let their_handshake: SenderHandshake = serde_json::from_slice(&their_handshake_raw.bytes)?;
    return Ok(Some(their_handshake));
}

async fn receive_stream_files(
    their_stream: &mut RecvStream,
    their_handshake: &SenderHandshake,
) -> Result<Vec<ReceiverFile>> {
    let vessel_paths = create_vessel_paths(their_handshake);
    loop {
        let mut chunk_len_bytes = [0u8; 2];
        let read_result = their_stream.read_exact(&mut chunk_len_bytes).await;
        if read_result.is_err() {
            break;
        }
        let chunk_len: u16 = u16::from_be_bytes(chunk_len_bytes);
        let mut chunk_bytes: Vec<u8> = vec![0u8; chunk_len as usize];
        their_stream.read_exact(&mut chunk_bytes).await?;
        let projection: FileProjection = serde_json::from_slice(&chunk_bytes)?;

        let vessel_path = vessel_paths
            .iter()
            .find(|vp| vp.file_name().unwrap().to_str().unwrap() == projection.id)
            .unwrap();
        if vessel_path.exists() {
            let mut file = std::fs::File::options().append(true).open(vessel_path)?;
            file.write_all(&projection.data)?;
            file.flush()?;
        } else {
            let mut file = std::fs::File::create(vessel_path)?;
            file.write_all(&projection.data)?;
            file.flush()?;
        }
    }
    return Ok(vessel_paths
        .iter()
        .map(|vp| {
            let id = vp.file_name().unwrap().to_str().unwrap();
            let handshake_file = their_handshake.files.iter().find(|f| f.id == id).unwrap();
            let data = Arc::new(ReceiverFileData::new(vp.to_path_buf()));
            return ReceiverFile {
                id: id.to_string(),
                name: handshake_file.name.clone(),
                data,
            };
        })
        .collect());
}

fn create_vessel_paths(their_handshake: &SenderHandshake) -> Vec<std::path::PathBuf> {
    let tmp_dir = std::env::temp_dir();
    let mut paths: Vec<std::path::PathBuf> = Vec::with_capacity(their_handshake.files.len());
    for f in &their_handshake.files {
        let path = tmp_dir.as_path().join(f.id.clone());
        paths.push(path);
    }
    return paths;
}

uniffi::include_scaffolding!("receiver");
