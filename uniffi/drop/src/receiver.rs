mod receive_files;

use std::sync::Arc;

pub use receive_files::*;

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
    inner: receiver::ReceiverFileData,
}
impl ReceiverFileData {
    pub fn len(&self) -> u64 {
        return self.inner.len();
    }

    pub fn read(&self) -> Option<u8> {
        return self.inner.read();
    }
}
