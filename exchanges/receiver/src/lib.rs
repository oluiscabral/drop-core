mod receive_files;

use std::{
    io::Read,
    os::unix::fs::FileExt,
    sync::{Arc, atomic::AtomicU64},
};

pub use receive_files::*;

#[derive(Debug, thiserror::Error)]
pub enum ReceiverError {
    #[error("TODO: \"{0}\".")]
    TODO(String),
}

pub struct ReceiverProfile {
    pub name: String,
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

uniffi::include_scaffolding!("receiver");
