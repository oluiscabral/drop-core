// TODO: DOUBLE CHECK ALL `unwrap`
// TODO: move interactors to different modules

use anyhow::Result;
use common::{FileProjection, HandshakeProfile, ReceiverHandshake, SenderHandshake};
use entities::Profile;
use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream},
};
use iroh_base::ticket::NodeTicket;
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::fs::FileExt,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64},
    },
};
use tokio::task::JoinHandle;
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
pub struct ReceiveFilesBubble {
    // TODO: derive debug
    profile: Profile,
    endpoint: Arc<Endpoint>,
    connection: Arc<Connection>,
    task: RwLock<Option<JoinHandle<Result<()>>>>,
    is_consumed: AtomicBool,
    is_running: Arc<AtomicBool>,
    is_finished: Arc<AtomicBool>,
    is_cancelled: Arc<AtomicBool>,
    files: Arc<RwLock<Vec<ReceiverFile>>>,
    subscribers: Arc<RwLock<HashMap<String, Arc<dyn ReceiveFilesSubscriber>>>>,
}
impl ReceiveFilesBubble {
    pub fn new(profile: Profile, endpoint: Endpoint, connection: Connection) -> Self {
        return Self {
            profile,
            endpoint: Arc::new(endpoint),
            connection: Arc::new(connection),
            task: RwLock::new(None),
            is_consumed: AtomicBool::new(false),
            is_running: Arc::new(AtomicBool::new(false)),
            is_finished: Arc::new(AtomicBool::new(false)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            files: Arc::new(RwLock::new(Vec::new())),
            subscribers: Arc::new(RwLock::new(HashMap::new())),
        };
    }

    pub fn start(&self) {
        let is_running = self.is_running.load(std::sync::atomic::Ordering::Acquire);
        let is_finished = self.is_finished.load(std::sync::atomic::Ordering::Acquire);
        let is_cancelled = self.is_cancelled.load(std::sync::atomic::Ordering::Acquire);
        if is_running || is_finished || is_cancelled {
            // TODO: handle exception
            todo!();
        }
        let endpoint = self.endpoint.clone();
        let is_running = self.is_running.clone();
        let is_finished = self.is_finished.clone();
        let is_cancelled = self.is_cancelled.clone();
        let files = self.files.clone();
        let profile = self.profile.clone();
        let connection = self.connection.clone();
        let subscribers = self.subscribers.clone();
        is_running.store(true, std::sync::atomic::Ordering::Release);
        let task: JoinHandle<Result<()>> = tokio::spawn(async {
            let (stream, their_stream) = connection.open_bi().await?;
            let mut carrier = Carrier {
                endpoint,
                is_running,
                is_finished,
                is_cancelled,
                profile,
                connection,
                stream,
                their_stream,
                files,
                their_handshake: None,
                subscribers,
            };
            send_handshake(&mut carrier).await?;
            receive_handshake(&mut carrier).await?;
            receive_stream_files(&mut carrier).await?;
            finish(&mut carrier).await?;
            return Ok(());
        });
        let _ = self.task.write().unwrap().insert(task);
        return ();
    }

    pub fn cancel(&self) {
        let is_running = self.is_running.load(std::sync::atomic::Ordering::Acquire);
        let is_finished = self.is_finished.load(std::sync::atomic::Ordering::Acquire);
        if !is_running || is_finished {
            return ();
        }
        return self
            .is_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        return self.is_finished.load(std::sync::atomic::Ordering::Acquire);
    }

    pub fn is_cancelled(&self) -> bool {
        return self.is_cancelled.load(std::sync::atomic::Ordering::Acquire);
    }

    pub fn get_files(&self) -> Vec<ReceiverFile> {
        let is_consumed = self.is_consumed.load(std::sync::atomic::Ordering::Relaxed);
        if is_consumed {
            // TODO: HANDLE EXCEPTION
            todo!();
        }
        let is_cancelled = self.is_cancelled.load(std::sync::atomic::Ordering::Relaxed);
        if is_cancelled {
            // TODO: HANDLE EXCEPTION
            todo!();
        }
        loop {
            let task = self.task.read().unwrap();
            if task.is_none() {
                break;
            }
            let task = task.as_ref().unwrap();
            if task.is_finished() {
                break;
            }
        }
        let is_finished = self.is_finished.load(std::sync::atomic::Ordering::Relaxed);
        if !is_finished {
            // TODO: handle exception
            todo!();
        }
        self.is_consumed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let mut files = self.files.write().unwrap();
        let len = files.len();
        return files.drain(0..len).collect();
    }

    pub fn subscribe(&self, subscriber: Arc<dyn ReceiveFilesSubscriber>) {
        self.subscribers
            .write()
            .unwrap()
            .insert(subscriber.get_id(), subscriber);
        return ();
    }

    pub fn unsubscribe(&self, subscriber: Arc<dyn ReceiveFilesSubscriber>) {
        self.subscribers
            .write()
            .unwrap()
            .remove(&subscriber.get_id());
        return ();
    }
}

struct Carrier {
    pub profile: Profile,
    pub endpoint: Arc<Endpoint>,
    pub connection: Arc<Connection>,
    pub stream: SendStream,
    pub their_stream: RecvStream,
    pub is_running: Arc<AtomicBool>,
    pub is_finished: Arc<AtomicBool>,
    pub is_cancelled: Arc<AtomicBool>,
    pub files: Arc<RwLock<Vec<ReceiverFile>>>,
    pub their_handshake: Option<SenderHandshake>,
    pub subscribers: Arc<RwLock<HashMap<String, Arc<dyn ReceiveFilesSubscriber>>>>,
}

async fn send_handshake(carrier: &mut Carrier) -> Result<()> {
    let handshake = ReceiverHandshake {
        profile: HandshakeProfile {
            id: carrier.profile.id.clone(),
            name: carrier.profile.name.clone(),
        },
    };
    let handshake_serialized = serde_json::to_vec(&handshake).unwrap();
    let handshake_serialized_len = handshake_serialized.len() as u32;
    let handshake_serialized_len_bytes = handshake_serialized_len.to_be_bytes();
    carrier
        .stream
        .write_all(&handshake_serialized_len_bytes)
        .await?;
    carrier.stream.write_all(&handshake_serialized).await?;
    return Ok(());
}

async fn receive_handshake(carrier: &mut Carrier) -> Result<()> {
    let mut their_handshake_len_raw = [0u8; 4];
    carrier
        .their_stream
        .read_exact(&mut their_handshake_len_raw)
        .await?;
    let their_handshake_len: u32 = u32::from_be_bytes(their_handshake_len_raw);
    let mut their_handshake_raw = vec![0u8; their_handshake_len as usize];
    carrier
        .their_stream
        .read_exact(&mut their_handshake_raw)
        .await?;
    let their_handshake: SenderHandshake = serde_json::from_slice(their_handshake_raw.as_slice())?;
    carrier.their_handshake = Some(their_handshake.clone());
    carrier
        .subscribers
        .read()
        .unwrap()
        .iter()
        .for_each(move |(_, s)| {
            s.notify_connecting(ReceiveFilesConnectingEvent {
                sender: ReceiveFilesProfile {
                    id: their_handshake.profile.id.clone(),
                    name: their_handshake.profile.name.clone(),
                },
            });
        });
    return Ok(());
}

async fn receive_stream_files(carrier: &mut Carrier) -> Result<()> {
    let their_handshake = carrier.their_handshake.as_ref().unwrap();
    let vessel_paths = create_vessel_paths(their_handshake);
    loop {
        let is_cancelled = carrier
            .is_cancelled
            .load(std::sync::atomic::Ordering::Relaxed);
        if is_cancelled {
            return Err(anyhow::Error::new(ReceiverError::TODO("".to_string())));
        }
        let mut chunk_len_bytes = [0u8; 2];
        let read_result = carrier.their_stream.read_exact(&mut chunk_len_bytes).await;
        if read_result.is_err() {
            break;
        }
        let chunk_len: u16 = u16::from_be_bytes(chunk_len_bytes);
        let mut chunk_bytes: Vec<u8> = vec![0u8; chunk_len as usize];
        carrier.their_stream.read_exact(&mut chunk_bytes).await?;
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

        let handshake_file = their_handshake
            .files
            .iter()
            .find(|f| f.id == projection.id)
            .unwrap();
        carrier
            .subscribers
            .read()
            .unwrap()
            .iter()
            .for_each(move |(_, s)| {
                s.notify_receiving(ReceiveFilesReceivingEvent {
                    id: handshake_file.id.clone(),
                    received: projection.data.len() as u64,
                });
            });
    }
    let files: Vec<ReceiverFile> = vessel_paths
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
        .collect();
    carrier.files.write().unwrap().extend(files);
    return Ok(());
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

async fn finish(carrier: &mut Carrier) -> Result<()> {
    carrier.stream.finish()?;
    carrier.stream.stopped().await?;
    carrier
        .connection
        .close(iroh::endpoint::VarInt::from_u32(0), &[0]);
    carrier.endpoint.close().await;
    carrier
        .is_running
        .store(false, std::sync::atomic::Ordering::Release);
    carrier
        .is_finished
        .store(false, std::sync::atomic::Ordering::Release);
    return Ok(());
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

pub trait ReceiveFilesSubscriber: Send + Sync {
    fn get_id(&self) -> String;
    fn notify_receiving(&self, event: ReceiveFilesReceivingEvent);
    fn notify_connecting(&self, event: ReceiveFilesConnectingEvent);
}

pub struct ReceiveFilesReceivingEvent {
    pub id: String,
    pub received: u64,
}

pub struct ReceiveFilesConnectingEvent {
    pub sender: ReceiveFilesProfile,
}

pub struct ReceiveFilesProfile {
    pub id: String,
    pub name: String,
}
// <- RESPONSE

pub async fn receive_files(
    request: ReceiveFilesRequest,
) -> Result<Arc<ReceiveFilesBubble>, ReceiverError> {
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
    return Ok(Arc::new(ReceiveFilesBubble::new(
        Profile {
            id: Uuid::new_v4().to_string(),
            name: request.profile.name,
        },
        endpoint,
        connection,
    )));
}

uniffi::include_scaffolding!("receiver");
