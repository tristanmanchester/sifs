use crate::daemon::paths::DaemonPaths;
use crate::daemon::protocol::{
    DAEMON_PROTOCOL_VERSION, DaemonError, DaemonRequest, DaemonRequestEnvelope,
    DaemonResponseEnvelope, ResultEnvelope,
};
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct DaemonClient {
    paths: DaemonPaths,
    timeout: Duration,
}

impl DaemonClient {
    pub fn new(paths: DaemonPaths) -> Self {
        Self {
            paths,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn send(&self, request: DaemonRequest) -> Result<crate::daemon::protocol::DaemonResult> {
        let request_id = format!("{}-{}", std::process::id(), now_nanos());
        let envelope = DaemonRequestEnvelope::new(request_id.clone(), request);
        let mut stream = UnixStream::connect(&self.paths.socket)
            .with_context(|| format!("connect SIFS daemon at {}", self.paths.socket.display()))?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        serde_json::to_writer(&mut stream, &envelope)?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        if line.trim().is_empty() {
            bail!("SIFS daemon closed the connection without a response");
        }
        let response: DaemonResponseEnvelope = serde_json::from_str(&line)?;
        if response.protocol_version != DAEMON_PROTOCOL_VERSION {
            bail!(
                "SIFS daemon protocol mismatch: client={}, daemon={}",
                DAEMON_PROTOCOL_VERSION,
                response.protocol_version
            );
        }
        if response.request_id != request_id {
            bail!(
                "SIFS daemon response id mismatch: expected {}, got {}",
                request_id,
                response.request_id
            );
        }
        match response.result {
            ResultEnvelope::Ok { result } => Ok(result),
            ResultEnvelope::Error { error } => Err(error_into_anyhow(error)),
        }
    }
}

fn error_into_anyhow(error: DaemonError) -> anyhow::Error {
    anyhow::anyhow!("{}: {}", error.code, error.message)
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
