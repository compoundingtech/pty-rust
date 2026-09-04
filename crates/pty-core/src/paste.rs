//! Bracketed paste (DECSET 2004) helpers, ported from the pty project's
//! `src/paste.ts`.
//!
//! When a receiving terminal has bracketed paste mode enabled, pasted text is
//! wrapped in these markers so applications can distinguish typed input from
//! pasted input (shells suppress history during paste; TUI agents treat the
//! block as one input event). `pty send --paste` wraps the whole payload so
//! multi-line prompts injected into agent sessions aren't submitted partway.

/// Sent BEFORE pasted content (CSI 200 ~).
pub const BRACKETED_PASTE_START: &str = "\x1b[200~";
/// Sent AFTER pasted content (CSI 201 ~).
pub const BRACKETED_PASTE_END: &str = "\x1b[201~";

/// Wrap `payload` in bracketed-paste START…END markers.
pub fn wrap_bracketed_paste(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + BRACKETED_PASTE_START.len() + BRACKETED_PASTE_END.len());
    out.extend_from_slice(BRACKETED_PASTE_START.as_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(BRACKETED_PASTE_END.as_bytes());
    out
}
