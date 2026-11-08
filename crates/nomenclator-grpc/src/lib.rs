//! Provides the library's operations over gRPC, in batches, for callers in other
//! languages. The service calls the same implementation that a Rust caller links. This
//! crate does not restate any rule.
//!
//! The contract is defined in `proto/nomenclator/v1/nomenclator.proto`.

use std::future::Future;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;

pub mod service;
pub mod wire;

/// The code generated from the contract. The `.proto` file documents its messages.
// The pedantic lints do not apply to generated code, which nobody edits.
#[allow(
    clippy::large_enum_variant,
    clippy::doc_markdown,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    missing_docs
)]
pub mod proto {
    tonic::include_proto!("nomenclator.v1");
}

pub use service::NamingService;

/// The protobuf package of the wire contract.
pub const WIRE_CONTRACT: &str = "nomenclator.v1";

pub use nomenclator::RULES_VERSION;

/// Returns the naming service as a server that can be added to a tonic router.
#[must_use]
pub fn naming_server() -> proto::nomenclator_server::NomenclatorServer<NamingService> {
    proto::nomenclator_server::NomenclatorServer::new(NamingService)
}

/// Serves the naming service on a bound TCP listener until `shutdown` completes.
pub async fn serve_tcp(
    listener: TcpListener,
    shutdown: impl Future<Output = ()>,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_service(naming_server())
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
}

/// Serves the naming service on a bound Unix socket listener until `shutdown` completes.
#[cfg(unix)]
pub async fn serve_unix(
    listener: tokio::net::UnixListener,
    shutdown: impl Future<Output = ()>,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_service(naming_server())
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::UnixListenerStream::new(listener),
            shutdown,
        )
        .await
}
