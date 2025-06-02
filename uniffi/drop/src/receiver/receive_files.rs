use std::sync::Arc;

use crate::DropError;

use super::{ReceiverFile, ReceiverFileData, ReceiverProfile};

pub struct ReceiveFilesRequest {
    pub ticket: String,
    pub confirmation: u8,
    pub profile: ReceiverProfile,
}

pub struct ReceiveFilesBubble {
    inner: receiver::ReceiveFilesBubble,
    _runtime: tokio::runtime::Runtime,
}
impl ReceiveFilesBubble {
    pub fn start(&self) {
        return self.inner.start();
    }

    pub fn cancel(&self) {
        return self.inner.cancel();
    }

    pub fn is_finished(&self) -> bool {
        return self.inner.is_finished();
    }

    pub fn is_cancelled(&self) -> bool {
        return self.inner.is_cancelled();
    }

    pub async fn get_files(&self) -> Result<Vec<ReceiverFile>, DropError> {
        return Ok(self
            .inner
            .get_files()
            .await
            .map_err(|e| DropError::TODO(e.to_string()))?
            .into_iter()
            .map(|f| ReceiverFile {
                id: f.id,
                name: f.name,
                data: Arc::new(ReceiverFileData { inner: f.data }),
            })
            .collect::<Vec<ReceiverFile>>());
    }

    pub fn subscribe(&self, subscriber: Arc<dyn ReceiveFilesSubscriber>) {
        let adapted_subscriber = ReceiveFilesSubscriberAdapter { inner: subscriber };
        return self.inner.subscribe(Arc::new(adapted_subscriber));
    }

    pub fn unsubscribe(&self, subscriber: Arc<dyn ReceiveFilesSubscriber>) {
        let adapted_subscriber = ReceiveFilesSubscriberAdapter { inner: subscriber };
        return self.inner.unsubscribe(Arc::new(adapted_subscriber));
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

struct ReceiveFilesSubscriberAdapter {
    inner: Arc<dyn ReceiveFilesSubscriber>,
}
impl receiver::ReceiveFilesSubscriber for ReceiveFilesSubscriberAdapter {
    fn get_id(&self) -> String {
        return self.inner.get_id();
    }

    fn notify_receiving(&self, event: receiver::ReceiveFilesReceivingEvent) {
        return self.inner.notify_receiving(ReceiveFilesReceivingEvent {
            id: event.id,
            received: event.received,
        });
    }

    fn notify_connecting(&self, event: receiver::ReceiveFilesConnectingEvent) {
        return self.inner.notify_connecting(ReceiveFilesConnectingEvent {
            sender: ReceiveFilesProfile {
                id: event.sender.id,
                name: event.sender.name,
            },
        });
    }
}

pub async fn receive_files(
    request: ReceiveFilesRequest,
) -> Result<Arc<ReceiveFilesBubble>, DropError> {
    let runtime = tokio::runtime::Runtime::new().map_err(|e| DropError::TODO(e.to_string()))?;
    let bubble = runtime
        .block_on(async {
            let adapted_request = create_adapted_request(request);
            return receiver::receive_files(adapted_request).await;
        })
        .map_err(|e| DropError::TODO(e.to_string()))?;
    return Ok(Arc::new(ReceiveFilesBubble {
        inner: bubble,
        _runtime: runtime,
    }));
}

fn create_adapted_request(request: ReceiveFilesRequest) -> receiver::ReceiveFilesRequest {
    let profile = receiver::ReceiverProfile {
        name: request.profile.name,
    };
    return receiver::ReceiveFilesRequest {
        profile,
        ticket: request.ticket,
        confirmation: request.confirmation,
    };
}
