// TODO: move interactors to different modules

mod send_files;

use entities::Data;
use std::{fmt::Debug, sync::Arc};

pub use send_files::*;

#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error("TODO: \"{0}\".")]
    TODO(String),
}

pub struct SenderProfile {
    pub name: String,
}

pub struct SenderFile {
    pub name: String,
    pub data: Arc<dyn SenderFileData>,
}

pub trait SenderFileData: Send + Sync {
    fn len(&self) -> u64;
    fn read(&self) -> Option<u8>;
}
struct SenderFileDataAdapter {
    data: Arc<dyn SenderFileData>,
}
impl Data for SenderFileDataAdapter {
    fn len(&self) -> u64 {
        return self.data.len();
    }

    fn read(&self) -> Option<u8> {
        return self.data.read();
    }
}

uniffi::include_scaffolding!("sender");
