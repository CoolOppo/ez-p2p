#![warn(clippy::all)]
// #![allow(unused)]

mod igd;

use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use color_eyre::eyre::Result;
use local_ip_address::local_ip;
use structopt::StructOpt;
use tokio::{
    fs,
    fs::File,
    io::{copy, AsyncWriteExt, BufReader, BufWriter},
    net::{TcpListener, TcpStream},
    spawn,
    time::timeout,
};

use crate::igd::{forward_port, IgdError};

#[derive(Debug, StructOpt)]
#[structopt(about = "Send or receive files. Receiving is default unless the send flag is used.")]
struct Opt {
    /// Specifies which file to send. (Default is receiving if this flag is
    /// unused.)
    #[structopt(short, long, name = "FILE", parse(from_os_str))]
    send: Option<PathBuf>,

    /// Peer's network address (ip:port)
    #[structopt(short = "o", long="ip", parse(try_from_str = parse_socket_addr))]
    peer_address: Option<SocketAddr>,
}

fn parse_socket_addr(src: &str) -> Result<SocketAddr, std::net::AddrParseError> {
    SocketAddr::from_str(src)
}

#[tokio::main]
async fn main() -> Result<()> {
    // tracing_subscriber::fmt()
    //     .with_max_level(Level::TRACE)
    //     .init();
    // color_eyre::install()?;
    let opt = Opt::from_args();

    if let Some(file) = opt.send {
        // sending
        if Path::exists(&file) && fs::metadata(&file).await?.is_file() {
            let tx = spawn(send(opt.peer_address.unwrap(), File::open(file).await?));
            tx.await??;
        } else {
            panic!("File does not exist.");
        }
    } else {
        // receiving
        let rx = spawn(receive());
        rx.await??;
    }

    Ok(())
}

async fn send(peer_addr: SocketAddr, file: File) -> Result<()> {
    println!("Sending to {:?}", peer_addr);
    let mut stream = TcpStream::connect(peer_addr).await?;
    copy(&mut BufReader::new(file), &mut stream).await?;
    stream.shutdown().await?;
    println!("Done!");

    Ok(())
}

async fn receive() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let mut internal_ip: IpAddr = local_ip().expect("Couldn't get internal IP.");

        for adapter in ipconfig::get_adapters()? {
            if adapter.friendly_name() == "Wi-Fi" {
                //this is most likely the network adapter we are looking for
                internal_ip = *adapter.ip_addresses().get(1).unwrap();
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let internal_ip: IpAddr = local_ip().expect("Couldn't get internal IP.");

    let port = port_scanner::request_open_port().expect("Unable to find an available port.");
    let external_ip = public_ip::addr().await;

    let local_addr = SocketAddr::new(internal_ip, port);

    let printed_addr = {
        if let Some(external_ip) = external_ip {
            SocketAddr::new(external_ip, port)
        } else {
            local_addr
        }
    };

    timeout(
        Duration::from_secs(30),
        forward_port(port, local_addr, Duration::from_secs(60 * 60)),
    )
    .await
    .map_err(|_| IgdError::TimedOut)??;
    println!("Endpoint created at {:?}\nListening...", printed_addr);

    let listener = TcpListener::bind(local_addr).await?;
    let (mut stream, _sender) = listener.accept().await?;
    let out_file = File::create("out.bin").await?;
    copy(&mut stream, &mut BufWriter::new(out_file)).await?;
    println!("\nTransfer complete!");
    Ok(())
}
