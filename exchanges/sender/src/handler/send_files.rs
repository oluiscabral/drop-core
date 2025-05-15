// TODO: HANDLE ERRORS

use anyhow::Result;
use common::{FileProjection, HandshakeFile, HandshakeProfile, ReceiverHandshake, SenderHandshake};
use entities::{File, Profile};
use iroh::{
    endpoint::{Connection, RecvStream, SendStream},
    protocol::ProtocolHandler,
};
use std::{
    collections::HashMap,
    fmt::Debug,
    pin::Pin,
    sync::{Arc, RwLock, atomic::AtomicBool},
};

#[derive(Clone, Debug)]
pub struct SendFilesHandlerResources {
    pub profile: Profile,
    pub files: Vec<File>,
}

pub trait SendFilesSubscriber: Send + Sync {
    fn get_id(&self) -> String;
    fn notify_sending(&self, event: SendFilesSendingEvent);
    fn notify_connecting(&self, event: SendFilesConnectingEvent);
}

pub struct SendFilesSendingEvent {
    pub name: String,
    pub sent: u64,
    pub remaining: u64,
}

pub struct SendFilesConnectingEvent {
    pub receiver: SendFilesProfile,
}

pub struct SendFilesProfile {
    pub id: String,
    pub name: String,
}

pub struct SendFilesHandler {
    resources: SendFilesHandlerResources,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    is_consumed: AtomicBool,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    is_finished: Arc<AtomicBool>,
    subscribers: RwLock<HashMap<String, Arc<dyn SendFilesSubscriber>>>,
}
impl Debug for SendFilesHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendFilesHandler")
            .field("resources", &self.resources)
            .field("is_consumed", &self.is_consumed)
            .field("is_finished", &self.is_finished)
            .finish()
    }
}
impl SendFilesHandler {
    pub fn new(resources: SendFilesHandlerResources) -> Self {
        return Self {
            resources,
            is_consumed: AtomicBool::new(false),
            is_finished: Arc::new(AtomicBool::new(false)),
            subscribers: RwLock::new(HashMap::new()),
        };
    }

    pub fn get_is_finished(&self) -> bool {
        return self.is_finished.load(std::sync::atomic::Ordering::Relaxed);
    }

    pub fn subscribe(&self, subscriber: Arc<dyn SendFilesSubscriber>) {
        self.subscribers
            .write()
            .unwrap()
            .insert(subscriber.get_id(), subscriber);
        return ();
    }

    pub fn unsubscribe(&self, subscriber: Arc<dyn SendFilesSubscriber>) {
        self.subscribers
            .write()
            .unwrap()
            .remove(&subscriber.get_id());
        return ();
    }
}
impl ProtocolHandler for SendFilesHandler {
    fn on_connecting(
        &self,
        connecting: iroh::endpoint::Connecting,
    ) -> Pin<Box<dyn Future<Output = Result<iroh::endpoint::Connection>> + Send + 'static>> {
        let is_consumed = self
            .is_consumed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .unwrap_or(true);
        if is_consumed {
            return Box::pin(async {
                return Err(anyhow::anyhow!("Connection has already been consumed."));
            });
        }
        return Box::pin(async {
            return Ok(connecting.await?);
        });
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        return Box::pin(async {});
    }

    fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
        let resources = self.resources.clone();
        let is_finished = self.is_finished.clone();
        let subscribers = self.subscribers.read().unwrap().clone();
        return Box::pin(async move {
            // TODO: RECEIVE THEIR HANDSHAKE
            let (stream, their_stream) = connection.accept_bi().await?;
            let mut carrier = Carrier {
                stream,
                connection,
                their_stream,
                resources,
                subscribers,
            };
            receive_handshake(&mut carrier).await?;
            send_handshake(&mut carrier).await?;
            send_files(&mut carrier).await?;
            finish(&mut carrier).await?;
            is_finished.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        });
    }
}

struct Carrier {
    pub connection: Connection,
    pub stream: SendStream,
    pub their_stream: RecvStream,
    pub resources: SendFilesHandlerResources,
    pub subscribers: HashMap<String, Arc<dyn SendFilesSubscriber>>,
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
    let their_handshake: ReceiverHandshake =
        serde_json::from_slice(their_handshake_raw.as_slice())?;
    carrier.subscribers.iter().for_each(|(_, s)| {
        s.notify_connecting(SendFilesConnectingEvent {
            receiver: SendFilesProfile {
                id: their_handshake.profile.id.clone(),
                name: their_handshake.profile.name.clone(),
            },
        });
    });
    return Ok(());
}

async fn send_handshake(carrier: &mut Carrier) -> Result<()> {
    let handshake = SenderHandshake {
        profile: HandshakeProfile {
            id: carrier.resources.profile.id.clone(),
            name: carrier.resources.profile.name.clone(),
        },
        files: carrier
            .resources
            .files
            .iter()
            .map(|f| HandshakeFile {
                id: f.id.clone(),
                name: f.name.clone(),
                len: f.data.len(),
            })
            .collect(),
    };
    let my_handshake_serialized = serde_json::to_vec(&handshake).unwrap();
    // TODO: LIMIT HOW MUCH FILES IT IS POSSIBLE TO SEND
    let my_handshake_serialized_len = my_handshake_serialized.len() as u32;
    let my_handshake_serialized_len_bytes = my_handshake_serialized_len.to_be_bytes();
    carrier
        .stream
        .write_all(&my_handshake_serialized_len_bytes)
        .await?;
    carrier.stream.write_all(&my_handshake_serialized).await?;
    return Ok(());
}

async fn send_files(carrier: &mut Carrier) -> Result<()> {
    // TODO: LIMIT CHUNK LEN **NEAR** U16, BECAUSE JSON TAKES MORE SPACE
    // TODO: OPTIMIZE
    let chunk_len: u16 = 1024; // TODO: FLEXIBILIZE CHUNK LEN
    for f in &carrier.resources.files {
        let mut is_finished = false;
        let mut sent = 0;
        let mut remaining = f.data.len();
        carrier.subscribers.iter().for_each(|(_, s)| {
            s.notify_sending(SendFilesSendingEvent {
                name: f.name.clone(),
                sent,
                remaining,
            });
        });
        loop {
            let mut chunk = Vec::new();
            for _ in 0..chunk_len {
                let b = f.data.read();
                if b.is_none() {
                    is_finished = true;
                    break;
                }
                chunk.push(b.unwrap());
            }
            let projection = FileProjection {
                id: f.id.clone(),
                data: chunk.to_vec(),
            };
            let serialized_projection = serde_json::to_vec(&projection).unwrap();
            let serialized_projection_len = serialized_projection.len() as u16;
            let serialized_projection_len_bytes = serialized_projection_len.to_be_bytes();
            carrier
                .stream
                .write_all(&serialized_projection_len_bytes)
                .await?;
            carrier.stream.write_all(&serialized_projection).await?;

            sent += chunk.len() as u64;
            remaining -= chunk.len() as u64;
            carrier.subscribers.iter().for_each(|(_, s)| {
                s.notify_sending(SendFilesSendingEvent {
                    name: f.name.clone(),
                    sent,
                    remaining,
                });
            });

            if is_finished {
                break;
            }
        }
    }
    return Ok(());
}

async fn finish(carrier: &mut Carrier) -> Result<()> {
    carrier.stream.finish()?;
    carrier.stream.stopped().await?;
    carrier
        .connection
        .close(iroh::endpoint::VarInt::from_u32(0), &[0]);
    return Ok(());
}
