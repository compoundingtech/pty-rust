//! Session registry: the on-disk layout under `$PTY_ROOT`, byte-compatible
//! with the Node project's `src/sessions.ts` so Rust and Node daemons and
//! CLIs can share one root.
//!
//! Per session: `<name>.json` (metadata), `<name>.sock`, `<name>.pid`,
//! `<name>.events.jsonl`, `<name>.lock`, `<name>.events.lock`, and
//! `.recovery/<name>.revision.json`. Atomic writes go through
//! `<path>.tmp.<pid>.<16 hex>` + rename; readers skip names containing
//! `.tmp.`.

pub mod atomic;
pub mod cleanup;
pub mod list;
pub mod lock;
pub mod metadata;
pub mod mutate;
pub mod names;
pub mod root;
pub mod tags;
pub mod time;

pub use atomic::{atomic_write, is_tmp_name, random_hex16};
pub use cleanup::{
    SessionGenerationOwner, cleanup, cleanup_all, cleanup_all_while_locked, cleanup_owned_all,
    cleanup_owned_socket, cleanup_socket, is_current_generation_owner,
};
pub use list::{
    DEFAULT_SOCKET_PROBE_BUDGET, ListOptions, SessionInfo, SessionStatus, all_session_names,
    ambiguous_reference_message, get_session, get_session_by_name, has_process_exited_for_reap,
    list_sessions, list_sessions_with, pid_alive, probe_sockets_within_budget, read_pid,
    read_pid_with, read_process_start_token, read_session_pid, resolve_ref, session_exists,
    socket_reachable, wait_for_process_exit,
};
pub use lock::{
    EVENT_LOCK_WAIT, LockBusy, LockGuard, LockRefusal, acquire_event_lock, acquire_file_lock,
    acquire_lock, event_busy_message, is_lock_owned_by_pid, lock_or_refusal,
    metadata_busy_message, release_event_lock, release_file_lock, release_lock, take_event_lock,
    take_metadata_lock, try_acquire_file_lock, wait_for_event_lock, with_both_locks,
};
pub use metadata::{
    EnvMap, SESSION_EXIT_LAST_LINES_LIMIT, SessionMetadata, TagMap, apply_metadata_diff,
    pretty_json, read_metadata, read_metadata_map, write_metadata, write_metadata_map,
    write_metadata_publication,
};
pub use mutate::{
    MetadataChangeSnapshot, MetadataPatch, MetadataPatchEvent, MetadataPatchResult, MutateOptions,
    MutateStatus, apply_metadata_patch_by_id, metadata_matches_observation,
    mutate_metadata_under_lock, mutate_metadata_under_lock_with, patch_metadata_by_id,
    set_display_name, update_tags,
};
pub use names::{
    SESSION_ID_ALPHABET, SESSION_ID_ATTEMPTS, auto_display_name, generate_id, random_session_name,
    sanitize_display_name, short_path, time_ago, time_ago_from_seconds, unique_id_failure_message,
    validate_display_name, validate_name,
};
pub use root::{
    SUN_PATH_MAX, default_session_dir, ensure_session_dir, event_lock_path, events_path, lock_path,
    metadata_path, pid_path, recovery_revision_path, root_length_check, session_dir, socket_path,
};
pub use tags::{
    EXACT_RESERVED_TAG_KEYS, GC_BOOKKEEPING_KEYS, KEEP_FALSEY, KEEP_TAG, extract_filter_tags,
    is_keep_requested, is_reserved_tag_key, matches_all_tags, reap_on_exit_default,
    should_reap_at_exit, strip_gc_bookkeeping,
};
pub use time::{
    iso8601, iso8601_from_epoch_ms, local_hms, now_epoch_ms, now_iso8601, parse_iso8601_ms,
};

/// Write a session's pid file (decimal, no newline) the way Node's daemon
/// does, after the socket is listening. Node's write is a plain
/// `writeFileSync`; the atomic temp+rename here is defence-in-depth so a
/// concurrent reader never catches an empty file.
///
/// node: src/server.ts:654
pub fn write_pid(name: &str, pid: u32) -> std::io::Result<()> {
    ensure_session_dir()?;
    atomic_write(&pid_path(name), pid.to_string().as_bytes())
}
