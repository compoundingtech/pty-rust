//! `pty stats`: the STATUS query, ported from `client.ts:344-389`.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::protocol::{MessageType, PacketReader, encode_status};
use crate::registry;
use crate::stats::StatsResult;
use crate::unix_connect::{self, Connect};

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

/// Query STATUS for many sessions from the calling thread: non-blocking
/// AF_UNIX connects in one poll(2) set, one shared deadline; EAGAIN (a full
/// accept queue) and EINPROGRESS are retried or awaited until the deadline.
///
/// Each session's result is what [`query_stats_in_with_timeout`] would return
/// for it, in `names` order, except that nothing waits past `deadline`: a
/// session whose daemon never accepts or never answers reports
/// [`ClientError::StatsTimeout`] instead of holding the caller.
pub fn query_stats_batch_in(
    root: &Path,
    names: &[String],
    deadline: Duration,
) -> Vec<(String, Result<StatsResult, ClientError>)> {
    let deadline = Instant::now() + deadline;
    let request = encode_status();
    let mut queries: Vec<BatchQuery> = names
        .iter()
        .map(|name| BatchQuery {
            path: root.join(format!("{name}.sock")),
            step: Step::Connect,
        })
        .collect();
    let mut polled: Vec<usize> = Vec::with_capacity(queries.len());
    let mut fds: Vec<libc::pollfd> = Vec::with_capacity(queries.len());
    let mut buf = [0u8; 8192];
    loop {
        polled.clear();
        fds.clear();
        let mut retrying = false;
        for (i, (query, name)) in queries.iter_mut().zip(names).enumerate() {
            if matches!(query.step, Step::Connect) {
                query.step = begin(&query.path, name, &request);
            }
            let (fd, events) = match &query.step {
                Step::Connect => {
                    retrying = true;
                    continue;
                }
                Step::Connecting(s) | Step::Writing(s, _) => (s.as_raw_fd(), libc::POLLOUT),
                Step::Reading(s, _) => (s.as_raw_fd(), libc::POLLIN),
                Step::Done(_) => continue,
            };
            polled.push(i);
            fds.push(libc::pollfd {
                fd,
                events,
                revents: 0,
            });
        }
        if fds.is_empty() && !retrying {
            break;
        }
        if !unix_connect::poll_until(&mut fds, deadline, retrying) {
            break;
        }
        for (pfd, &i) in fds.iter().zip(&polled) {
            if pfd.revents == 0 {
                continue;
            }
            let name = &names[i];
            let query = &mut queries[i];
            query.step = match std::mem::replace(&mut query.step, Step::Connect) {
                Step::Connecting(s) => match s.take_error() {
                    Ok(None) => write_request(s, 0, &request, name, &query.path),
                    Ok(Some(e)) | Err(e) => Step::Done(Box::new(Err(map_io_error(
                        name,
                        false,
                        GoneSet::Strict,
                        "connect",
                        Some(&query.path),
                        &e,
                    )))),
                },
                Step::Writing(s, written) => write_request(s, written, &request, name, &query.path),
                Step::Reading(s, reader) => read_response(s, reader, &mut buf, name),
                other => other,
            };
        }
    }
    queries
        .into_iter()
        .zip(names)
        .map(|(query, name)| {
            let result = match query.step {
                Step::Done(result) => *result,
                _ => Err(ClientError::StatsTimeout(name.clone())),
            };
            (name.clone(), result)
        })
        .collect()
}

struct BatchQuery {
    path: PathBuf,
    step: Step,
}

/// Where one batched STATUS query stands. Every stream is non-blocking.
enum Step {
    /// Not connected yet: the first attempt, or a retry after EAGAIN.
    Connect,
    /// EINPROGRESS; POLLOUT reports the outcome.
    Connecting(UnixStream),
    /// Connected, `usize` request bytes written.
    Writing(UnixStream, usize),
    /// Request sent; collecting packets until STATUS.
    Reading(UnixStream, PacketReader),
    Done(Box<Result<StatsResult, ClientError>>),
}

fn begin(path: &Path, name: &str, request: &[u8]) -> Step {
    match unix_connect::connect(path) {
        Connect::Connected(s) => write_request(s, 0, request, name, path),
        Connect::InProgress(s) => Step::Connecting(s),
        Connect::Busy => Step::Connect,
        Connect::Failed(e) => Step::Done(Box::new(Err(map_io_error(
            name,
            false,
            GoneSet::Strict,
            "connect",
            Some(path),
            &e,
        )))),
    }
}

fn write_request(
    mut socket: UnixStream,
    mut written: usize,
    request: &[u8],
    name: &str,
    path: &Path,
) -> Step {
    while written < request.len() {
        match socket.write(&request[written..]) {
            Ok(0) => {
                let e = io::Error::from(io::ErrorKind::WriteZero);
                return Step::Done(Box::new(Err(map_io_error(
                    name,
                    false,
                    GoneSet::Strict,
                    "write",
                    Some(path),
                    &e,
                ))));
            }
            Ok(n) => written += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Step::Writing(socket, written);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                return Step::Done(Box::new(Err(map_io_error(
                    name,
                    false,
                    GoneSet::Strict,
                    "write",
                    Some(path),
                    &e,
                ))));
            }
        }
    }
    Step::Reading(socket, PacketReader::new())
}

/// Drain what is readable now. The outcomes mirror
/// [`query_status_json_at`] read for read.
fn read_response(
    mut socket: UnixStream,
    mut reader: PacketReader,
    buf: &mut [u8],
    name: &str,
) -> Step {
    loop {
        match socket.read(buf) {
            Ok(0) => return Step::Done(Box::new(Err(ClientError::StatsTimeout(name.to_string())))),
            Ok(n) => match reader.feed(&buf[..n]) {
                Ok(packets) => {
                    if let Some(p) = packets.iter().find(|p| p.type_ == MessageType::Status) {
                        let json = String::from_utf8_lossy(&p.payload);
                        return Step::Done(Box::new(
                            serde_json::from_str(&json)
                                .map_err(|_| ClientError::InvalidStats(name.to_string())),
                        ));
                    }
                }
                Err(e) => {
                    let _ = std::io::stderr().write_all(dropping_connection_line(&e).as_bytes());
                    return Step::Done(Box::new(Err(ClientError::StatsTimeout(name.to_string()))));
                }
            },
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Step::Reading(socket, reader);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                return Step::Done(Box::new(Err(map_io_error(
                    name,
                    false,
                    GoneSet::Strict,
                    "read",
                    None,
                    &e,
                ))));
            }
        }
    }
}

/// The raw STATUS payload (the daemon's JSON, verbatim — what `stats --json`
/// prints). Not-found uses the strict gone set (ENOENT/ECONNREFUSED only).
pub fn query_status_json(name: &str, timeout: Duration) -> Result<String, ClientError> {
    let path = registry::socket_path(name);
    query_status_json_at(&path, name, timeout)
}

fn query_status_json_at(path: &Path, name: &str, timeout: Duration) -> Result<String, ClientError> {
    let deadline = Instant::now() + timeout;
    let mut socket = connect_session_at(path, name, GoneSet::Strict)?;
    socket
        .write_all(&encode_status())
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
