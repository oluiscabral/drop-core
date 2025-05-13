// TODO: HANDLE ERRORS

use anyhow::Result;
use common::{FileProjection, HandshakeFile, HandshakeProfile, SenderHandshake};
use entities::{File, Profile};
use iroh::{
    endpoint::{Connection, SendStream},
    protocol::ProtocolHandler,
};
use std::{
    fmt::Debug,
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Debug)]
pub struct SendFilesHandlerResources {
    pub profile: Profile,
    pub files: Vec<File>,
}

#[derive(Debug)]
pub struct SendFilesHandler {
    resources: SendFilesHandlerResources,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    is_consumed: AtomicBool,
    // TODO: POSSIBLY USE `Mutex<Option<bool>>` instead?
    is_finished: Arc<AtomicBool>,
}

impl SendFilesHandler {
    pub fn new(resources: SendFilesHandlerResources) -> Self {
        return Self {
            resources,
            is_consumed: AtomicBool::new(false),
            is_finished: Arc::new(AtomicBool::new(false)),
        };
    }

    pub fn get_is_finished(&self) -> bool {
        return self.is_finished.load(std::sync::atomic::Ordering::Relaxed);
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
        return Box::pin(async { Ok(connecting.await?) });
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
        return Box::pin(async move {
            // TODO: RECEIVE THEIR HANDSHAKE
            let (mut stream, _) = connection.accept_bi().await?;
            send_handshake(&mut stream, &resources).await?;
            send_files(&mut stream, &resources).await?;
            finish(&mut stream, &connection).await?;
            is_finished.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        });
    }
}

async fn send_handshake(
    stream: &mut SendStream,
    resources: &SendFilesHandlerResources,
) -> Result<()> {
    let handshake = SenderHandshake {
        profile: HandshakeProfile {
            id: resources.profile.id.clone(),
            name: resources.profile.name.clone(),
        },
        files: resources
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
    stream.write_all(&my_handshake_serialized_len_bytes).await?;
    stream.write_all(&my_handshake_serialized).await?;
    return Ok(());
}

async fn send_files(stream: &mut SendStream, resources: &SendFilesHandlerResources) -> Result<()> {
    // TODO: LIMIT CHUNK LEN **NEAR** U16, BECAUSE JSON TAKES MORE SPACE
    // TODO: OPTIMIZE
    let chunk_len: u16 = 1024; // TODO: FLEXIBILIZE CHUNK LEN
    for f in &resources.files {
        let mut is_finished = false;
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
            stream.write_all(&serialized_projection_len_bytes).await?;
            stream.write_all(&serialized_projection).await?;
            if is_finished {
                break;
            }
        }
    }
    return Ok(());
}

async fn finish(stream: &mut SendStream, connection: &Connection) -> Result<()> {
    stream.finish()?;
    stream.stopped().await?;
    connection.close(iroh::endpoint::VarInt::from_u32(0), &[0]);
    return Ok(());
}
