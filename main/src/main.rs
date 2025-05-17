use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::FileExt,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, atomic::AtomicU64},
};

use anyhow::Result;
use receiver::{
    ReceiveFilesConnectingEvent, ReceiveFilesReceivingEvent, ReceiveFilesRequest,
    ReceiveFilesSubscriber, ReceiverProfile, receive_files,
};
use sender::{
    SendFilesConnectingEvent, SendFilesRequest, SendFilesSendingEvent, SendFilesSubscriber,
    SenderFile, SenderFileData, SenderProfile, send_files,
};
use uuid::Uuid;

struct CustomSendFilesSubscriber;
impl SendFilesSubscriber for CustomSendFilesSubscriber {
    fn get_id(&self) -> String {
        return Uuid::new_v4().to_string();
    }

    fn notify_sending(&self, event: SendFilesSendingEvent) {
        println!("SENDER SendFilesSendingEvent");
        println!("name: {}", event.name);
        println!("remaining: {}", event.remaining);
        println!("sent: {}", event.sent);
        println!("=====================================");
    }

    fn notify_connecting(&self, event: SendFilesConnectingEvent) {
        println!("SENDER SendFilesConnectingEvent");
        println!("receiver SendFilesProfile");
        println!("id: {}", event.receiver.id);
        println!("name: {}", event.receiver.name);
        println!("=====================================");
    }
}

struct CustomReceiveFilesSubscriber;
impl ReceiveFilesSubscriber for CustomReceiveFilesSubscriber {
    fn get_id(&self) -> String {
        return Uuid::new_v4().to_string();
    }

    fn notify_receiving(&self, event: ReceiveFilesReceivingEvent) {
        println!("RECEIVER ReceiveFilesRecevingEvent");
        println!("id: {}", event.id);
        println!("received: {}", event.received);
        println!("=====================================");
    }

    fn notify_connecting(&self, event: ReceiveFilesConnectingEvent) {
        println!("RECEIVER ReceiveFilesConnectingEvent");
        println!("receiver ReceiveFilesProfile");
        println!("id: {}", event.sender.id);
        println!("name: {}", event.sender.name);
        println!("=====================================");
    }
}

struct CustomSenderFileData {
    offset: AtomicU64,
    path: PathBuf,
}
impl CustomSenderFileData {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: AtomicU64::new(0),
        }
    }
}
impl SenderFileData for CustomSenderFileData {
    fn len(&self) -> u64 {
        let file = std::fs::File::open(self.path.to_path_buf()).unwrap();
        return file.bytes().count() as u64;
    }

    fn read(&self) -> Option<u8> {
        let offset = self.offset.load(std::sync::atomic::Ordering::Acquire);
        if offset >= self.len() {
            return None;
        }
        let file = std::fs::File::open(self.path.to_path_buf()).unwrap();
        let mut buf = [0u8; 1];
        let read_result = file.read_exact_at(&mut buf, offset);
        if read_result.is_err() {
            return None;
        }
        return Some(buf[0]);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let arg_slices = arg_refs.as_slice();

    if arg_slices[0] == "send" {
        run_send_files(arg_slices[1..].to_vec()).await?;
        return Ok(());
    } else if arg_slices[0] == "receive" {
        run_receive_files(arg_slices[1..].to_vec()).await?;
        return Ok(());
    } else {
        on_invalid(args);
    }
    Ok(())
}

async fn run_send_files(args: Vec<&str>) -> Result<()> {
    let file_paths: Vec<PathBuf> = args.iter().map(|s| PathBuf::from(s)).collect();
    if file_paths.len() == 0 {
        println!("Cannot send an empty list of files!");
        return Ok(());
    }
    let request = SendFilesRequest {
        files: create_sender_files(file_paths),
        profile: get_sender_profile(),
    };
    let bubble = send_files(request).await?;
    let subscriber = CustomSendFilesSubscriber {};
    bubble.subscribe(Arc::new(subscriber));
    println!("ticket: \"{}\"", bubble.get_ticket());
    println!("confirmation: \"{}\"", bubble.get_confirmation());
    tokio::signal::ctrl_c().await?;
    bubble.cancel().await;
    return Ok(());
}

fn create_sender_files(paths: Vec<PathBuf>) -> Vec<SenderFile> {
    return paths
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_str().unwrap();
            let data = CustomSenderFileData::new(p.to_path_buf());
            return SenderFile {
                name: name.to_string(),
                data: Arc::new(data),
            };
        })
        .collect();
}

fn get_sender_profile() -> SenderProfile {
    return SenderProfile {
        name: String::from("sender-cli"),
    };
}

async fn run_receive_files(args: Vec<&str>) -> Result<()> {
    if args.len() != 3 {
        println!("Couldn't parse receive command line arguments: {args:?}");
        println!("Usage:");
        println!("    # to receive:");
        println!("    cargo run main -- receive [OUTPUT] [TICKET] [CONFIRMATION]");
        return Ok(());
    }
    let ticket = args[1].to_string();
    let confirmation = u8::from_str(args[2])?;
    let profile = get_receiver_profile();

    let arg_path = PathBuf::from(args[0]);
    let receiving_path = arg_path.to_path_buf().join(ticket.clone());
    fs::create_dir(receiving_path.to_path_buf())?;

    let request = ReceiveFilesRequest {
        ticket,
        confirmation,
        profile,
    };
    let bubble = receive_files(request).await?;

    let subscriber = CustomReceiveFilesSubscriber {};
    bubble.subscribe(Arc::new(subscriber));

    bubble.start();
    let files = bubble.get_files().await?;
    for f in files {
        let mut file = fs::File::create(receiving_path.to_path_buf().join(f.name))?;
        loop {
            let b = f.data.read();
            if b.is_none() {
                break;
            }
            file.write(&[b.unwrap()])?;
        }
    }
    return Ok(());
}

fn get_receiver_profile() -> ReceiverProfile {
    return ReceiverProfile {
        name: String::from("receiver-cli"),
    };
}

fn on_invalid(args: Vec<String>) {
    println!("Couldn't parse command line arguments: {args:?}");
    println!("Usage:");
    println!("    # to send:");
    println!("    cargo run main -- send [SOURCE...]");
    println!("    # this will print a ticket and a confirmation code.");
    println!();
    println!("    # to receive:");
    println!("    cargo run main -- receive [OUTPUT] [TICKET] [CONFIRMATION]");
}
