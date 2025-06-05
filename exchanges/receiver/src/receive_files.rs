use anyhow::{Ok, Result};
use common::{FileProjection, HandshakeProfile, ReceiverHandshake, SenderHandshake};
use entities::Profile;
use iroh::{
    Endpoint,
    endpoint::{ApplicationClose, Connection, ConnectionError, RecvStream, SendStream, VarInt},
};
use iroh_base::ticket::NodeTicket;
use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, RwLock, atomic::AtomicBool},
};
use uuid::Uuid;

use crate::{ReceiverFile, ReceiverFileData, ReceiverProfile};

pub struct ReceiveFilesRequest {
    pub ticket: String,
    pub confirmation: u8,
    pub profile: ReceiverProfile,
}

pub struct ReceiveFilesBubble {
    profile: Profile,
    endpoint: Endpoint,
    connection: Connection,
    their_handshake: RwLock<Option<SenderHandshake>>,
    is_running: AtomicBool,
    is_consumed: AtomicBool,
    is_finished: AtomicBool,
    is_cancelled: AtomicBool,
    subscribers: RwLock<HashMap<String, Arc<dyn ReceiveFilesSubscriber>>>,
}
impl ReceiveFilesBubble {
    pub fn new(profile: Profile, endpoint: Endpoint, connection: Connection) -> Self {
        return Self {
            profile,
            endpoint: endpoint,
            connection: connection,
            their_handshake: RwLock::new(None),
            is_running: AtomicBool::new(false),
            is_consumed: AtomicBool::new(false),
            is_finished: AtomicBool::new(false),
            is_cancelled: AtomicBool::new(false),
            subscribers: RwLock::new(HashMap::new()),
        };
    }

    pub async fn start(&self) -> Result<Vec<ReceiverFile>> {
        if self.is_running() || self.is_consumed() || self.is_finished() {
            return Err(anyhow::Error::msg(
                "Already running or has run or finished.",
            ));
        }
        self.is_running
            .store(true, std::sync::atomic::Ordering::Release);
        self.is_consumed
            .store(true, std::sync::atomic::Ordering::Release);

        self.greet().await?;
        let files_result = self.receive_files().await;

        self.endpoint.close().await;
        self.is_running
            .store(false, std::sync::atomic::Ordering::Release);

        if files_result.is_ok() {
            self.is_finished
                .store(true, std::sync::atomic::Ordering::Release);
        }

        return files_result;
    }

    async fn greet(&self) -> Result<()> {
        let mut bi = self.connection.open_bi().await?;
        self.send_handshake(&mut bi).await?;
        self.receive_handshake(&mut bi).await?;
        bi.0.finish()?;
        bi.0.stopped().await?;
        return Ok(());
    }

    async fn send_handshake(&self, bi: &mut (SendStream, RecvStream)) -> Result<()> {
        let handshake = ReceiverHandshake {
            profile: HandshakeProfile {
                id: self.profile.id.clone(),
                name: self.profile.name.clone(),
            },
        };
        let serialized_handshake = serde_json::to_vec(&handshake).unwrap();
        let serialized_handshake_len = serialized_handshake.len() as u32;
        let serialized_handshake_header = serialized_handshake_len.to_be_bytes();
        bi.0.write_all(&serialized_handshake_header).await?;
        bi.0.write_all(&serialized_handshake).await?;
        return Ok(());
    }

    async fn receive_handshake(&self, bi: &mut (SendStream, RecvStream)) -> Result<()> {
        let mut serialized_handshake_header = [0u8; 4];
        bi.1.read_exact(&mut serialized_handshake_header).await?;
        let serialized_handshake_len = u32::from_be_bytes(serialized_handshake_header);
        let mut serialized_handshake = vec![0u8; serialized_handshake_len as usize];
        bi.1.read_exact(&mut serialized_handshake).await?;
        let handshake: SenderHandshake = serde_json::from_slice(&serialized_handshake)?;
        self.their_handshake
            .write()
            .unwrap()
            .replace(handshake.clone());
        self.subscribers
            .read()
            .unwrap()
            .iter()
            .for_each(move |(_, s)| {
                s.notify_connecting(ReceiveFilesConnectingEvent {
                    sender: ReceiveFilesProfile {
                        id: handshake.profile.id.clone(),
                        name: handshake.profile.name.clone(),
                    },
                });
            });
        return Ok(());
    }

    async fn receive_files(&self) -> Result<Vec<ReceiverFile>> {
        let their_handshake = self
            .their_handshake
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let vessel_paths = self.create_vessel_paths();
        loop {
            if self.is_cancelled() {
                self.connection.close(
                    VarInt::from_u32(0),
                    String::from("Receive files has been cancelled.").as_bytes(),
                );
                return Err(anyhow::Error::msg("Receive files has been cancelled."));
            }
            let uni_result = self.connection.accept_uni().await;
            if uni_result.is_err() {
                let err = uni_result.unwrap_err();
                if err.eq(&ConnectionError::ApplicationClosed(ApplicationClose {
                    error_code: VarInt::from_u32(200),
                    reason: String::from("Finished.").into(),
                })) {
                    break;
                }
                return Err(anyhow::Error::msg("Connection unexpectedly closed."));
            }
            let mut uni = uni_result.unwrap();
            let projection = self.read_next_projection(&mut uni).await?;
            if projection.is_none() {
                break;
            }
            let projection = projection.unwrap();
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
            self.subscribers
                .read()
                .unwrap()
                .iter()
                .for_each(move |(_, s)| {
                    s.notify_receiving(ReceiveFilesReceivingEvent {
                        id: projection.id.clone(),
                        received: projection.data.len() as u64,
                    });
                });
        }
        let files: Vec<ReceiverFile> = vessel_paths
            .iter()
            .map(|vp| {
                let id = vp.file_name().unwrap().to_str().unwrap();
                let handshake_file = their_handshake.files.iter().find(|f| f.id == id).unwrap();
                let data = ReceiverFileData::new(vp.to_path_buf());
                return ReceiverFile {
                    id: id.to_string(),
                    name: handshake_file.name.clone(),
                    data,
                };
            })
            .collect();
        return Ok(files);
    }

    fn create_vessel_paths(&self) -> Vec<std::path::PathBuf> {
        let their_handshake = self
            .their_handshake
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        let tmp_dir = std::env::temp_dir();
        let mut paths: Vec<std::path::PathBuf> = Vec::with_capacity(their_handshake.files.len());
        for f in &their_handshake.files {
            let path = tmp_dir.as_path().join(f.id.clone());
            paths.push(path);
        }
        return paths;
    }

    async fn read_next_projection(&self, uni: &mut RecvStream) -> Result<Option<FileProjection>> {
        let serialized_projection_len = self.read_serialized_projection_len(uni).await?;
        if serialized_projection_len.is_none() {
            return Ok(None);
        }
        let mut serialized_projection = vec![0u8; serialized_projection_len.unwrap()];
        uni.read_exact(&mut serialized_projection).await?;
        let projection: FileProjection = serde_json::from_slice(&serialized_projection)?;
        return Ok(Some(projection));
    }

    async fn read_serialized_projection_len(&self, uni: &mut RecvStream) -> Result<Option<usize>> {
        let mut serialized_projection_header = [0u8; 2];
        let read = uni.read(&mut serialized_projection_header).await?;
        if read.is_none() {
            return Ok(None);
        }
        if read.unwrap() != 2 {
            return Err(anyhow::Error::msg("Invalid data chunk length."));
        }
        let serialized_projection_len =
            u16::from_be_bytes(serialized_projection_header[..2].try_into().unwrap());
        return Ok(Some(serialized_projection_len as usize));
    }

    pub fn cancel(&self) {
        if !self.is_running() || self.is_finished() {
            return ();
        }
        return self
            .is_cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn is_running(&self) -> bool {
        return self.is_running.load(std::sync::atomic::Ordering::Acquire);
    }

    fn is_consumed(&self) -> bool {
        return self.is_consumed.load(std::sync::atomic::Ordering::Acquire);
    }

    pub fn is_finished(&self) -> bool {
        return self.is_finished.load(std::sync::atomic::Ordering::Acquire);
    }

    pub fn is_cancelled(&self) -> bool {
        return self.is_cancelled.load(std::sync::atomic::Ordering::Relaxed);
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

pub async fn receive_files(request: ReceiveFilesRequest) -> Result<ReceiveFilesBubble> {
    let ticket: NodeTicket = request.ticket.parse()?;
    let endpoint = Endpoint::builder().discovery_n0().bind().await?;
    let connection = endpoint.connect(ticket, &[request.confirmation]).await?;
    return Ok(ReceiveFilesBubble::new(
        Profile {
            id: Uuid::new_v4().to_string(),
            name: request.profile.name,
        },
        endpoint,
        connection,
    ));
}
