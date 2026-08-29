//! Per-connection state and the inbound packet handlers, one for one with
//! Node's `handleClient` switch.
//!
//! A client is in one of three roles — command (`pty send`, `pty stats`:
//! never attached), writable (ATTACH) or readonly (PEEK) — and, after an
//! ATTACH or PEEK, waits in `Settling` for its SCREEN cut. A settling client
//! receives no DATA and no EXIT: every byte the child produces meanwhile is
//! parsed into the terminal and lands in the SCREEN. The cut is synchronous
//! on the actor thread, so there is no window between "what the SCREEN
//! shows" and "what the next DATA continues from".
//!
//! node: src/server.ts:75-90, 904-1063, 1213-1267

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use pty_core::protocol::{
    Packet, MessageType, decode_peek, decode_size, encode_exit, encode_geometry, encode_screen,
    encode_status_response,
};
use pty_core::registry::{self, MutateOptions};
use pty_terminal::{Range, SerializeOpts};

use super::lifecycle::Daemon;

/// Node's `REDRAW_SETTLE_MS`: how long after a resize the child gets to
/// redraw before an attacher's SCREEN is cut.
pub const REDRAW_SETTLE: Duration = Duration::from_millis(80);

/// Bytes to a client's socket, or an instruction to end/destroy it.
pub enum Out {
    Bytes(Vec<u8>),
    /// `socket.end()`: half-close, the peer closes when it is done.
    End,
    /// `socket.destroy()`: close both ways now.
    Destroy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// `attachSeq === 0 && !readonly`: never sent ATTACH or PEEK.
    Command,
    Writable,
    Readonly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutKind {
    Attach { size_matched: bool },
    Peek { plain: bool, full: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Live,
    Settling {
        deadline: Instant,
        generation: u64,
        kind: CutKind,
    },
}

pub struct Client {
    pub tx: Sender<Out>,
    pub role: Role,
    pub rows: u16,
    pub cols: u16,
    /// `attachSeq`: the negotiation sequence of the last ATTACH/RESIZE.
    pub attach_seq: u64,
    /// `initialScreenGeneration`: bumped by every ATTACH/PEEK so an older
    /// pending cut is superseded.
    pub generation: u64,
    pub phase: Phase,
}

impl Client {
    pub fn new(tx: Sender<Out>, rows: u16, cols: u16) -> Client {
        Client {
            tx,
            role: Role::Command,
            rows,
            cols,
            attach_seq: 0,
            generation: 0,
            phase: Phase::Live,
        }
    }

    pub fn send(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Out::Bytes(bytes));
    }

    pub fn is_settling(&self) -> bool {
        matches!(self.phase, Phase::Settling { .. })
    }

    /// Node's `broadcastGeometry` set: attached (ever) or readonly.
    pub fn gets_geometry(&self) -> bool {
        self.attach_seq > 0 || self.role == Role::Readonly
    }

    /// Node's `negotiateSize` set: writable with a negotiation sequence.
    pub fn constrains_size(&self) -> bool {
        self.role == Role::Writable && self.attach_seq > 0
    }
}

impl Daemon {
    /// A packet from client `id`.
    pub(crate) fn on_packet(&mut self, id: u64, packet: Packet) {
        match packet.type_ {
            MessageType::Attach => self.on_attach(id, &packet.payload),
            MessageType::Peek => self.on_peek(id, &packet.payload),
            MessageType::Data => self.on_data(id, &packet.payload),
            MessageType::Resize => self.on_resize(id, &packet.payload),
            MessageType::Detach => self.on_detach(id),
            MessageType::Status => self.on_status(id),
            _ => {}
        }
    }

    /// node: src/server.ts:931-996
    fn on_attach(&mut self, id: u64, payload: &[u8]) {
        if payload.len() < 4 {
            return;
        }
        if !self.clients.contains_key(&id) {
            return;
        }
        let (rows, cols) = decode_size(payload);
        // Read before negotiation: a smaller client shrinks the session to
        // its own size, which would then look like it had matched.
        let size_matched = rows == self.actor.rows() && cols == self.actor.cols();
        self.attach_counter += 1;
        let generation = {
            let c = self.clients.get_mut(&id).expect("checked");
            c.role = Role::Writable;
            c.rows = rows;
            c.cols = cols;
            c.attach_seq = self.attach_counter;
            c.generation += 1;
            c.generation
        };
        let resized = self.negotiate_size();
        if !resized {
            let g = encode_geometry(self.actor.rows(), self.actor.cols());
            self.clients[&id].send(g);
        }
        // Best-effort: a concurrent metadata command wins this stamp, but
        // neither writer can overwrite the other's snapshot.
        let _ = registry::mutate_metadata_under_lock(
            &self.name,
            |m| {
                m.last_attach_at = Some(registry::now_iso8601());
                true
            },
            &MutateOptions {
                expected_generation: Some(self.generation.clone()),
                expected_metadata: None,
            },
        );
        let delay = if !self.exited {
            let since_last = self
                .last_resize
                .map(|t| t.elapsed())
                .unwrap_or(Duration::MAX);
            if resized {
                Some(self.settle)
            } else if since_last < self.settle {
                Some(self.settle - since_last)
            } else {
                None
            }
        } else {
            None
        };
        self.schedule_cut(id, generation, CutKind::Attach { size_matched }, delay);
    }

    /// node: src/server.ts:998-1020
    fn on_peek(&mut self, id: u64, payload: &[u8]) {
        if !self.clients.contains_key(&id) {
            return;
        }
        let generation = {
            let c = self.clients.get_mut(&id).expect("checked");
            c.role = Role::Readonly;
            c.generation += 1;
            c.generation
        };
        let resized = self.negotiate_size();
        if !resized {
            let g = encode_geometry(self.actor.rows(), self.actor.cols());
            self.clients[&id].send(g);
        }
        let (plain, full) = decode_peek(payload);
        self.schedule_cut(id, generation, CutKind::Peek { plain, full }, None);
    }

    /// node: src/server.ts:1022-1027
    fn on_data(&mut self, id: u64, payload: &[u8]) {
        let Some(c) = self.clients.get(&id) else {
            return;
        };
        if !self.exited && c.role != Role::Readonly {
            self.write_pty(payload);
        }
    }

    /// node: src/server.ts:1029-1038
    fn on_resize(&mut self, id: u64, payload: &[u8]) {
        let Some(c) = self.clients.get_mut(&id) else {
            return;
        };
        if c.role != Role::Writable || c.attach_seq == 0 || payload.len() < 4 {
            return;
        }
        let (rows, cols) = decode_size(payload);
        c.rows = rows;
        c.cols = cols;
        self.attach_counter += 1;
        c.attach_seq = self.attach_counter;
        self.negotiate_size();
    }

    /// node: src/server.ts:1040-1043
    fn on_detach(&mut self, id: u64) {
        if let Some(c) = self.clients.get(&id) {
            let _ = c.tx.send(Out::End);
        }
    }

    /// node: src/server.ts:1045-1049
    fn on_status(&mut self, id: u64) {
        let json = serde_json::to_string(&self.collect_stats()).unwrap_or_else(|_| "{}".into());
        if let Some(c) = self.clients.get(&id) {
            c.send(encode_status_response(&json));
        }
    }

    /// `close` / `error`: forget the socket and renegotiate.
    ///
    /// node: src/server.ts:1054-1062
    pub(crate) fn on_closed(&mut self, id: u64) {
        if self.clients.remove(&id).is_some() {
            self.negotiate_size();
        }
    }

    /// Arm (or perform, when `delay` is `None`) the SCREEN cut for `id`.
    fn schedule_cut(&mut self, id: u64, generation: u64, kind: CutKind, delay: Option<Duration>) {
        let deadline = Instant::now() + delay.unwrap_or(Duration::ZERO);
        if let Some(c) = self.clients.get_mut(&id) {
            c.phase = Phase::Settling {
                deadline,
                generation,
                kind,
            };
        }
        if delay.is_none() {
            self.cut(id);
        }
    }

    /// Node's `beginInitialScreenCut` callback: SCREEN from the live
    /// terminal, then live, then EXIT when the child is already gone, then
    /// the redraw nudge for an attacher whose size differed.
    ///
    /// node: src/server.ts:1213-1252
    pub(crate) fn cut(&mut self, id: u64) {
        let Some(c) = self.clients.get(&id) else {
            return;
        };
        let Phase::Settling {
            generation, kind, ..
        } = c.phase
        else {
            return;
        };
        if generation != c.generation {
            return;
        }
        let screen = match kind {
            CutKind::Attach { .. } => self.actor.serialize(SerializeOpts::ATTACH),
            CutKind::Peek { plain: true, full } => self.actor.plain(if full {
                Range::Full
            } else {
                Range::Viewport
            }),
            CutKind::Peek { plain: false, full } => self.actor.serialize(if full {
                SerializeOpts::PEEK_FULL
            } else {
                SerializeOpts::PEEK
            }),
        };
        let c = self.clients.get_mut(&id).expect("checked");
        c.send(encode_screen(screen.as_bytes()));
        c.phase = Phase::Live;
        if self.exited {
            c.send(encode_exit(self.exit_code));
        }
        if let CutKind::Attach { size_matched } = kind
            && !self.exited
            && !size_matched
        {
            self.nudge_redraw();
        }
    }

    /// Every settling client whose deadline has passed gets its cut.
    pub(crate) fn service_cuts(&mut self, now: Instant) {
        let due: Vec<u64> = self
            .clients
            .iter()
            .filter_map(|(id, c)| match c.phase {
                Phase::Settling { deadline, .. } if deadline <= now => Some(*id),
                _ => None,
            })
            .collect();
        for id in due {
            self.cut(id);
        }
    }

    /// The earliest pending cut deadline.
    pub(crate) fn next_cut_deadline(&self) -> Option<Instant> {
        self.clients
            .values()
            .filter_map(|c| match c.phase {
                Phase::Settling { deadline, .. } => Some(deadline),
                Phase::Live => None,
            })
            .min()
    }

    /// Node's `broadcast` for DATA and EXIT: live clients only; settling
    /// clients see the bytes in their SCREEN (and an EXIT after it).
    ///
    /// node: src/server.ts:1255-1267
    pub(crate) fn broadcast(&self, packet: &[u8]) {
        for c in self.clients.values() {
            if c.is_settling() {
                continue;
            }
            c.send(packet.to_vec());
        }
    }
}
