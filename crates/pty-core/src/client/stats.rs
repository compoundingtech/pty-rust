//! `pty stats`: the STATUS query, ported from `client.ts:344-389`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::protocol::{MessageType, PacketReader, encode_status};
use crate::registry;
use crate::stats::StatsResult;

use super::{ClientError, GoneSet, connect_session_with, dropping_connection_line, map_io_error};

/// `queryStats` budget (`client.ts:344`).
pub const STATS_TIMEOUT: Duration = Duration::from_secs(2);

/// Query live stats from a running session (2 s budget).
pub fn query_stats(name: &str) -> Result<StatsResult, ClientError> {
    query_stats_with_timeout(name, STATS_TIMEOUT)
}

/// [`query_stats`] with an explicit budget.
pub fn query_stats_with_timeout(name: &str, timeout: Duration) -> Result<StatsResult, ClientError> {
    let json = query_status_json(name, timeout)?;
    serde_json::from_str(&json).map_err(|_| ClientError::InvalidStats(name.to_string()))
}

/// The raw STATUS payload (the daemon's JSON, verbatim — what `stats --json`
/// prints). Not-found uses the strict gone set (ENOENT/ECONNREFUSED only).
pub fn query_status_json(name: &str, timeout: Duration) -> Result<String, ClientError> {
    let deadline = Instant::now() + timeout;
    let mut socket = connect_session_with(name, GoneSet::Strict)?;
    let path = registry::socket_path(name);
    socket
        .write_all(&encode_status())
        .map_err(|e| map_io_error(name, false, GoneSet::Strict, "write", Some(&path), &e))?;
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
