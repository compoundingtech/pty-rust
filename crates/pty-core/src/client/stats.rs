//! `pty stats`: the STATUS query, ported from `client.ts:344-389`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::protocol::{AttachedClient, MessageType, PacketReader, encode_status, encode_status_clients};
use crate::registry;
use crate::stats::StatsResult;

use super::{ClientError, GoneSet, connect_session_at, dropping_connection_line, map_io_error};

/// `queryStats` budget (`client.ts:344`).
pub const STATS_TIMEOUT: Duration = Duration::from_secs(2);

/// Query live stats from a running session (2 s budget).
pub fn query_stats(name: &str) -> Result<StatsResult, ClientError> {
    query_stats_with_timeout(name, STATS_TIMEOUT)
}

/// [`query_stats`] with an explicit budget.
pub fn query_stats_with_timeout(name: &str, timeout: Duration) -> Result<StatsResult, ClientError> {
    let path = registry::socket_path(name);
    query_stats_at(&path, name, timeout)
}

/// Query live stats from `root/<name>.sock` (2 s budget).
pub fn query_stats_in(root: &Path, name: &str) -> Result<StatsResult, ClientError> {
    query_stats_in_with_timeout(root, name, STATS_TIMEOUT)
}

/// [`query_stats_in`] with an explicit budget.
pub fn query_stats_in_with_timeout(
    root: &Path,
    name: &str,
    timeout: Duration,
) -> Result<StatsResult, ClientError> {
    query_stats_at(&root.join(format!("{name}.sock")), name, timeout)
}

fn query_stats_at(
    socket_path: &Path,
    name: &str,
    timeout: Duration,
) -> Result<StatsResult, ClientError> {
    let json = query_status_json_at(socket_path, name, timeout)?;
    serde_json::from_str(&json).map_err(|_| ClientError::InvalidStats(name.to_string()))
}

/// The raw STATUS payload (the daemon's JSON, verbatim — what `stats --json`
/// prints). Not-found uses the strict gone set (ENOENT/ECONNREFUSED only).
pub fn query_status_json(name: &str, timeout: Duration) -> Result<String, ClientError> {
    let path = registry::socket_path(name);
    query_status_json_at(&path, name, timeout)
}

fn query_status_json_at(path: &Path, name: &str, timeout: Duration) -> Result<String, ClientError> {
    query_status_at(path, name, timeout, &encode_status())
}

/// List attached clients without changing the existing stats response.
/// A legacy daemon returns stats instead of an array; its clients are unknown.
pub fn query_attached_clients(path: &Path, name: &str) -> Vec<AttachedClient> {
    query_status_at(path, name, Duration::from_millis(500), &encode_status_clients())
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn query_status_at(
    path: &Path,
    name: &str,
    timeout: Duration,
    request: &[u8],
) -> Result<String, ClientError> {
    let deadline = Instant::now() + timeout;
    let mut socket = connect_session_at(path, name, GoneSet::Strict)?;
    socket
        .write_all(request)
        .map_err(|e| map_io_error(name, false, GoneSet::Strict, "write", Some(path), &e))?;
    let mut reader = PacketReader::new();
    let mut buf = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ClientError::StatsTimeout(name.to_string()));
        }
        let _ = socket.set_read_timeout(Some(remaining));
        match socket.read(&mut buf) {
            // Closed without a STATUS: Node has no close handler here, so the
            // 2 s timer is what fires.
            Ok(0) => return Err(ClientError::StatsTimeout(name.to_string())),
            Ok(n) => match reader.feed(&buf[..n]) {
                Ok(packets) => {
                    for p in packets {
                        if p.type_ == MessageType::Status {
                            return Ok(String::from_utf8_lossy(&p.payload).into_owned());
                        }
                    }
                }
                Err(e) => {
                    let _ = std::io::stderr().write_all(dropping_connection_line(&e).as_bytes());
                    return Err(ClientError::StatsTimeout(name.to_string()));
                }
            },
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(ClientError::StatsTimeout(name.to_string()));
            }
            Err(e) => {
                return Err(map_io_error(name, false, GoneSet::Strict, "read", None, &e));
            }
        }
    }
}

/// Query STATUS over an already-connected socket (used by the conformance
/// rig against a routed or scripted daemon).
pub fn query_status_json_over(
    mut socket: UnixStream,
    name: &str,
    timeout: Duration,
) -> Result<String, ClientError> {
    let deadline = Instant::now() + timeout;
    socket
        .write_all(&encode_status())
        .map_err(|e| map_io_error(name, false, GoneSet::Strict, "write", None, &e))?;
    let mut reader = PacketReader::new();
    let mut buf = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ClientError::StatsTimeout(name.to_string()));
        }
        let _ = socket.set_read_timeout(Some(remaining));
        match socket.read(&mut buf) {
            Ok(0) => return Err(ClientError::StatsTimeout(name.to_string())),
            Ok(n) => {
                for p in reader.feed(&buf[..n]).unwrap_or_default() {
                    if p.type_ == MessageType::Status {
                        return Ok(String::from_utf8_lossy(&p.payload).into_owned());
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Err(ClientError::StatsTimeout(name.to_string()));
            }
            Err(e) => return Err(map_io_error(name, false, GoneSet::Strict, "read", None, &e)),
        }
    }
}
