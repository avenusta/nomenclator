//! Serves the naming service over gRPC on a TCP address or a Unix socket. The server
//! keeps no state and stops on Ctrl-C.

use clap::{ArgGroup, Parser};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about)]
#[command(group(ArgGroup::new("listen_address").required(true).args(["tcp", "unix"])))]
struct Arguments {
    /// The TCP address to listen on, such as `127.0.0.1:50051`. Port 0 selects a free
    /// port. Either `--tcp` or `--unix` is required.
    #[arg(long)]
    tcp: Option<SocketAddr>,
    /// The Unix socket path to listen on. The file must not exist. Either `--tcp` or
    /// `--unix` is required.
    #[arg(long)]
    unix: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments = Arguments::parse();
    let shutdown = async {
        // If the signal handler cannot be installed, the server runs until the process is
        // killed.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    if let Some(address) = arguments.tcp {
        let listener = tokio::net::TcpListener::bind(address).await?;
        // Prints one line so that a caller that launches the server can read the chosen
        // port.
        println!("listening on tcp:{}", listener.local_addr()?);
        nomenclator_grpc::serve_tcp(listener, shutdown).await?;
    } else if let Some(socket_path) = arguments.unix {
        serve_on_unix_socket(&socket_path, shutdown).await?;
    }
    Ok(())
}

#[cfg(unix)]
async fn serve_on_unix_socket(
    socket_path: &std::path::Path,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::UnixListener::bind(socket_path)?;
    println!("listening on unix:{}", socket_path.display());
    nomenclator_grpc::serve_unix(listener, shutdown).await?;
    // The socket file remains after the listener closes, so a clean stop removes it to
    // leave no stale file.
    std::fs::remove_file(socket_path)?;
    Ok(())
}

#[cfg(not(unix))]
async fn serve_on_unix_socket(
    _socket_path: &std::path::Path,
    _shutdown: impl std::future::Future<Output = ()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("--unix: Unix sockets are not available on this platform. Use --tcp".into())
}
