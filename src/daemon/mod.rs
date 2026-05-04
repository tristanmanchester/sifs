pub mod client;
pub mod manager;
pub mod paths;
pub mod protocol;
pub mod runtime;

pub use client::DaemonClient;
pub use manager::{IndexManager, IndexManagerStatus};
pub use paths::{DaemonPaths, default_daemon_paths};
pub use protocol::{
    DAEMON_PROTOCOL_VERSION, DaemonError, DaemonRequest, DaemonRequestEnvelope,
    DaemonResponseEnvelope, DaemonResult, DaemonStatus, IndexIdentity, IndexRuntimeOptions,
    SearchOptionsWire, SourceKind, SourceSpec, daemon_version,
};
pub use runtime::{DaemonRuntimeOptions, run_foreground};
