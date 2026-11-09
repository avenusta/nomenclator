//! Serves the naming service over gRPC on a TCP address or a Unix socket. The server
//! keeps no state and stops on Ctrl-C. With `--stop-on-stdin-close`, it also stops when
//! its standard input closes, so a host that launches it as a child process ends it
//! however the host itself ends.

use clap::{ArgGroup, Parser};
use std::io::Read;
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
    /// Stops the server when its standard input closes, in addition to Ctrl-C. A host
    /// that launches the server as a child process with a piped standard input sets
    /// this, so the server ends however the host ends. Defaults to off.
    #[arg(long)]
    stop_on_stdin_close: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let arguments = Arguments::parse();
    let ctrl_c = async {
        // If the signal handler cannot be installed, Ctrl-C does not stop the server.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let stdin_closed = stdin_closed(arguments.stop_on_stdin_close);
    let shutdown = async {
        tokio::select! {
            () = ctrl_c => {}
            () = stdin_closed => {}
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

/// Completes when standard input closes, if `enabled`. Never completes otherwise.
async fn stdin_closed(enabled: bool) {
    if !enabled {
        std::future::pending::<()>().await;
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // A plain thread, not a blocking task of the runtime: the runtime waits for its
    // blocking tasks when it shuts down, and a read of standard input ends only when
    // the input closes.
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0u8; 1024];
        // Reads and discards the input until it ends or fails.
        while matches!(stdin.read(&mut buffer), Ok(1..)) {}
        // The receiver is gone only if the server is stopping already.
        let _ = sender.send(());
    });
    // The sender is dropped only when the thread ends, which is when the input closed.
    let _ = receiver.await;
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
