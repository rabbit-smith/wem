//! wem-server binary: a standalone wwise.v1 gRPC endpoint.
//!
//! Address resolution (first match wins):
//!   1. `--addr <host:port>` CLI argument
//!   2. `WEM_GRPC_ADDR` environment variable
//!   3. `127.0.0.1:50051`
//!
//! Prints `READY <addr>` once the listener is bound (the mock server's
//! ready-line convention, so harnesses can share their polling logic).

use std::error::Error;
use std::net::SocketAddr;

use tonic::transport::server::TcpIncoming;
use tonic::transport::Server;
use wem_server::service::WemEncoderService;
use wem_server::wwise::wem_encoder_server::WemEncoderServer;

fn resolve_addr() -> SocketAddr {
    let args: Vec<String> = std::env::args().collect();
    let from_cli = args
        .iter()
        .position(|arg| arg == "--addr")
        .and_then(|index| args.get(index + 1))
        .cloned();
    from_cli
        .or_else(|| std::env::var("WEM_GRPC_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:50051".to_string())
        .parse()
        .expect("invalid --addr / WEM_GRPC_ADDR: expected host:port")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let addr = resolve_addr();
    let incoming = TcpIncoming::bind(addr)?;
    let bound_addr = incoming.local_addr()?;
    println!("READY {bound_addr}");
    Server::builder()
        .add_service(WemEncoderServer::new(WemEncoderService::new()))
        .serve_with_incoming(incoming)
        .await?;
    Ok(())
}
