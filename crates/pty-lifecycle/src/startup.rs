//! Daemon-owned startup lease values and host-boot monotonic deadlines.
//!
//! node: src/startup-lease.ts

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Public startup lease options carried in `PTY_SERVER_CONFIG`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupLeaseOptions {
    pub timeout_ms: u64,
    pub lifecycle_tag: String,
}

/// The generation-fenced lifecycle value published before the launcher returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedStartupLease {
    pub generation: String,
    pub lifecycle_tag: String,
    pub boot_id: String,
    pub deadline_monotonic_ns: u128,
    pub starting_value: String,
}

/// Why a startup lease became terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupLeaseTerminalCause {
    Exit,
    Deadline,
    TeardownUnavailable,
}

impl StartupLeaseTerminalCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "exit",
            Self::Deadline => "deadline",
            Self::TeardownUnavailable => "teardown-unavailable",
        }
    }
}

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_TIMER_DELAY_MS: u64 = 2_147_483_647;

/// Arm a positive startup budget once in the current boot's monotonic domain.
pub fn arm_startup_lease(
    options: &StartupLeaseOptions,
    generation: &str,
) -> Result<ArmedStartupLease, String> {
    if options.timeout_ms == 0 || options.timeout_ms > MAX_SAFE_INTEGER {
        return Err("startup lease timeoutMs must be a positive safe integer".to_string());
    }
    if options.lifecycle_tag.is_empty() {
        return Err("startup lease lifecycleTag must not be empty".to_string());
    }
    let boot_id = read_boot_identity()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "startup lease boot identity is unavailable".to_string())?;
    let now = monotonic_now_ns()
        .ok_or_else(|| "startup lease monotonic clock is unavailable".to_string())?;
    let budget = u128::from(options.timeout_ms) * 1_000_000;
    let deadline_monotonic_ns = now + budget;
    let starting_value = serde_json::json!({
        "_tag": "starting",
        "generation": generation,
        "bootId": boot_id,
        "deadlineMonotonicNs": deadline_monotonic_ns.to_string(),
    })
    .to_string();
    Ok(ArmedStartupLease {
        generation: generation.to_string(),
        lifecycle_tag: options.lifecycle_tag.clone(),
        boot_id,
        deadline_monotonic_ns,
        starting_value,
    })
}

pub fn terminal_startup_lease_value(generation: &str, cause: StartupLeaseTerminalCause) -> String {
    serde_json::json!({
        "_tag": "terminal",
        "generation": generation,
        "cause": cause.as_str(),
    })
    .to_string()
}

pub fn startup_lease_deadline_cause(containment_complete: bool) -> StartupLeaseTerminalCause {
    if containment_complete {
        StartupLeaseTerminalCause::Deadline
    } else {
        StartupLeaseTerminalCause::TeardownUnavailable
    }
}

pub fn remaining_lease_delay(deadline_monotonic_ns: u128) -> Duration {
    let Some(now) = monotonic_now_ns() else {
        return Duration::ZERO;
    };
    let remaining_ns = deadline_monotonic_ns.saturating_sub(now);
    let remaining_ms = remaining_ns.div_ceil(1_000_000);
    Duration::from_millis(
        u64::try_from(remaining_ms)
            .unwrap_or(u64::MAX)
            .min(MAX_TIMER_DELAY_MS),
    )
}

pub fn monotonic_now_ns() -> Option<u128> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `value` points to writable storage for one `timespec`.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut value) } != 0 {
        return None;
    }
    let seconds = u128::try_from(value.tv_sec).ok()?;
    let nanos = u128::try_from(value.tv_nsec).ok()?;
    Some(seconds * 1_000_000_000 + nanos)
}

pub fn read_boot_identity() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        let value = value.trim();
        return (!value.is_empty()).then(|| format!("linux:{value}"));
    }
    #[cfg(target_os = "macos")]
    {
        let name = c"kern.boottime";
        let mut value: libc::timeval = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::timeval>();
        // SAFETY: the name is NUL-terminated; `value` and `len` describe a
        // writable `timeval`, and no replacement value is supplied.
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut libc::timeval).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || len != std::mem::size_of::<libc::timeval>()
        {
            return None;
        }
        return Some(format!(
            "darwin:{{ sec = {}, usec = {} }}",
            value.tv_sec, value.tv_usec
        ));
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_values_match_node_json() {
        assert_eq!(
            terminal_startup_lease_value("generation", StartupLeaseTerminalCause::Deadline),
            r#"{"_tag":"terminal","generation":"generation","cause":"deadline"}"#
        );
        assert_eq!(
            startup_lease_deadline_cause(false),
            StartupLeaseTerminalCause::TeardownUnavailable
        );
    }
}
