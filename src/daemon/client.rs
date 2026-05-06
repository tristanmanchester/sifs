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
        configure_stream_timeout(&stream, self.timeout)?;
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

fn configure_stream_timeout(stream: &UnixStream, timeout: Duration) -> Result<()> {
    if timeout.is_zero() {
        return Ok(());
    }
    stream
        .set_read_timeout(Some(timeout))
        .with_context(|| format!("set daemon socket read timeout to {timeout:?}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .with_context(|| format!("set daemon socket write timeout to {timeout:?}"))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::configure_stream_timeout;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    #[test]
    fn zero_timeout_is_explicitly_left_unset() {
        let (stream, _peer) = UnixStream::pair().unwrap();

        configure_stream_timeout(&stream, Duration::ZERO).unwrap();

        assert_eq!(stream.read_timeout().unwrap(), None);
        assert_eq!(stream.write_timeout().unwrap(), None);
    }

    #[test]
    fn nonzero_timeout_is_applied() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let timeout = Duration::from_millis(250);

        configure_stream_timeout(&stream, timeout).unwrap();

        assert_eq!(stream.read_timeout().unwrap(), Some(timeout));
        assert_eq!(stream.write_timeout().unwrap(), Some(timeout));
    }
}
