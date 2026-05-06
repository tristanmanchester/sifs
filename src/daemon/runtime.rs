use crate::daemon::manager::IndexManager;
use crate::daemon::paths::DaemonPaths;
use crate::daemon::protocol::{
    DAEMON_PROTOCOL_VERSION, DaemonError, DaemonRequest, DaemonRequestEnvelope,
    DaemonResponseEnvelope, DaemonResult, DaemonStatus, IndexStatusResult, ResultEnvelope,
    SearchResultSet, daemon_version,
};
use crate::utils::resolve_chunk;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct DaemonRuntimeOptions {
    pub paths: DaemonPaths,
    pub replace_existing_socket: bool,
}

pub fn run_foreground(options: DaemonRuntimeOptions) -> Result<()> {
    options.paths.ensure_runtime_dir()?;
    prepare_socket(&options.paths, options.replace_existing_socket)?;
    std::fs::write(&options.paths.pid_file, std::process::id().to_string())
        .with_context(|| format!("write daemon pid file {}", options.paths.pid_file.display()))?;
    let listener = UnixListener::bind(&options.paths.socket)
        .with_context(|| format!("bind SIFS daemon socket {}", options.paths.socket.display()))?;
    let mut manager = IndexManager::new();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(stream, &mut manager) {
                    eprintln!("sifs daemon connection error: {error:#}");
                }
            }
            Err(error) => eprintln!("sifs daemon accept error: {error}"),
        }
    }
    Ok(())
}

fn prepare_socket(paths: &DaemonPaths, replace_existing_socket: bool) -> Result<()> {
    if !paths.socket.exists() {
        return Ok(());
    }
    if replace_existing_socket {
        std::fs::remove_file(&paths.socket)
            .with_context(|| format!("remove old daemon socket {}", paths.socket.display()))?;
        return Ok(());
    }
    let metadata = std::fs::metadata(&paths.socket)
        .with_context(|| format!("inspect daemon socket path {}", paths.socket.display()))?;
    if !metadata.file_type().is_socket() {
        bail!(
            "SIFS daemon socket path already exists but is not a socket: {}",
            paths.socket.display()
        );
    }
    match UnixStream::connect(&paths.socket) {
        Ok(_) => {
            bail!(
                "SIFS daemon socket already exists at {} and appears active. Stop the running daemon or pass --replace-existing-socket.",
                paths.socket.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            std::fs::remove_file(&paths.socket).with_context(|| {
                format!("remove stale daemon socket {}", paths.socket.display())
            })?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "probe existing SIFS daemon socket at {}",
                paths.socket.display()
            )
        }),
    }
}

fn handle_connection(mut stream: UnixStream, manager: &mut IndexManager) -> Result<()> {
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    if line.trim().is_empty() {
        return Ok(());
    }
    let response = match serde_json::from_str::<DaemonRequestEnvelope>(&line) {
        Ok(request) => handle_request(request, manager),
        Err(error) => DaemonResponseEnvelope::error(
            "invalid",
            DaemonError::new("invalid_json", error.to_string()),
        ),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn handle_request(
    envelope: DaemonRequestEnvelope,
    manager: &mut IndexManager,
) -> DaemonResponseEnvelope {
    if envelope.protocol_version != DAEMON_PROTOCOL_VERSION {
        return DaemonResponseEnvelope::error(
            envelope.request_id,
            DaemonError::new(
                "protocol_mismatch",
                format!(
                    "client protocol {}, daemon protocol {}",
                    envelope.protocol_version, DAEMON_PROTOCOL_VERSION
                ),
            ),
        );
    }
    match execute_request(envelope.request, manager) {
        Ok(result) => DaemonResponseEnvelope::ok(envelope.request_id, result),
        Err(error) => DaemonResponseEnvelope {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            request_id: envelope.request_id,
            result: ResultEnvelope::Error {
                error: DaemonError::new("request_failed", error.to_string()),
            },
        },
    }
}

fn execute_request(request: DaemonRequest, manager: &mut IndexManager) -> Result<DaemonResult> {
    match request {
        DaemonRequest::Ping => Ok(DaemonResult::Pong {
            version: daemon_version().to_owned(),
        }),
        DaemonRequest::Status => Ok(DaemonResult::Status(DaemonStatus {
            version: daemon_version().to_owned(),
            protocol_version: DAEMON_PROTOCOL_VERSION,
            pid: std::process::id(),
            indexes: manager.status().indexes,
        })),
        DaemonRequest::IndexStatus { source, options } => {
            let index = manager.get(source.clone(), options)?;
            Ok(DaemonResult::IndexStatus(IndexManager::index_status(
                index, source,
            )))
        }
        DaemonRequest::Search {
            source,
            options,
            query,
            search,
        } => {
            let started = Instant::now();
            let mode = search.mode;
            let search_options = search.into();
            let index = manager.get(source.clone(), options)?;
            let results = index.search_with(&query, &search_options)?;
            Ok(DaemonResult::Search(SearchResultSet {
                source,
                query,
                mode,
                stats: index.stats(),
                elapsed_ms: elapsed_ms(started),
                results,
                warnings: index.warnings().to_vec(),
            }))
        }
        DaemonRequest::FindRelated {
            source,
            options,
            file_path,
            line,
            top_k,
        } => {
            let started = Instant::now();
            let index = manager.get(source.clone(), options)?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                bail!("No chunk found at {file_path}:{line}");
            };
            let results = index.find_related(&chunk, top_k)?;
            Ok(DaemonResult::FindRelated(SearchResultSet {
                source,
                query: format!("{file_path}:{line}"),
                mode: crate::types::SearchMode::Semantic,
                stats: index.stats(),
                elapsed_ms: elapsed_ms(started),
                results,
                warnings: index.warnings().to_vec(),
            }))
        }
        DaemonRequest::ListFiles {
            source,
            options,
            limit,
        } => {
            let index = manager.get(source.clone(), options)?;
            let files = index.indexed_files();
            Ok(DaemonResult::ListFiles {
                source,
                total: files.len(),
                files: files.into_iter().take(limit).collect(),
            })
        }
        DaemonRequest::GetChunk {
            source,
            options,
            file_path,
            line,
        } => {
            let index = manager.get(source.clone(), options)?;
            let Some(chunk) = resolve_chunk(&index.chunks, &file_path, line) else {
                bail!("No chunk found at {file_path}:{line}");
            };
            Ok(DaemonResult::GetChunk { source, chunk })
        }
        DaemonRequest::Refresh { source, options } => {
            let index = manager.refresh(source.clone(), options)?;
            Ok(DaemonResult::Refresh(IndexStatusResult {
                source,
                stats: index.stats(),
                semantic_loaded: index.semantic_loaded(),
                warnings: index.warnings().to_vec(),
            }))
        }
        DaemonRequest::Clear { source, options } => {
            let removed = manager.clear(source.clone(), options);
            Ok(DaemonResult::Clear { source, removed })
        }
    }
}

#[allow(dead_code)]
fn is_socket_path(path: &Path) -> bool {
    path.exists()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::prepare_socket;
    use crate::daemon::paths::DaemonPaths;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    #[test]
    fn prepare_socket_reclaims_stale_socket_through_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.sock");
        let link = temp.path().join("linked.sock");
        drop(UnixListener::bind(&target).unwrap());
        symlink(&target, &link).unwrap();
        let paths = DaemonPaths {
            runtime_dir: temp.path().to_path_buf(),
            socket: link.clone(),
            pid_file: temp.path().join("sifs.pid"),
            log_file: temp.path().join("sifs.log"),
        };

        prepare_socket(&paths, false).unwrap();

        assert!(!link.exists());
    }
}
