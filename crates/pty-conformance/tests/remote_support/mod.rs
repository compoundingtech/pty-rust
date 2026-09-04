//! Shared scaffolding for the `--remote` suites: a stub `fabric` on PATH and
//! an out-of-process bridge that plays fabric's `expose --exec` role.
//!
//! `fabric dial <peer> pty-remote` (the stub) prints the path of a local Unix
//! socket. Behind that socket a small python3 helper accepts connections and,
//! per connection, spawns the binary under test's own `pty remote-serve
//! --stdio` with `PTY_ROOT` pointed at the "remote" registry, splicing the
//! socket to the handler's stdin/stdout (exactly the exec-bridge in
//! tests/remote-exec-bridge.test.ts). The helper is also the killable tunnel
//! of tests/remote-reconnect.test.ts: a `drop` file severs every live tunnel
//! and a `block` file makes new dials fail while the listener stays up.

#![allow(dead_code)]

use pty_conformance::*;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The bridge: `bridge.py <listen_sock> <pty_bin> <srv_root> <ctrl_dir> <pid_file>`.
const BRIDGE_PY: &str = r#"
import os, signal, socket, subprocess, sys, threading, time

listen, pty_bin, srv_root, ctrl, pidfile = sys.argv[1:6]
with open(pidfile, "w") as f:
    f.write(str(os.getpid()) + "\n")

lock = threading.Lock()
pairs = []  # [client socket, handler process]

def write_active():
    tmp = os.path.join(ctrl, "active.tmp")
    with open(tmp, "w") as f:
        f.write(str(len(pairs)) + "\n")
    os.replace(tmp, os.path.join(ctrl, "active"))

def remove(pair):
    with lock:
        if pair in pairs:
            pairs.remove(pair)
            write_active()

def sever(pair):
    client, proc = pair
    try:
        client.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    try:
        client.close()
    except OSError:
        pass
    try:
        proc.kill()
    except OSError:
        pass

def handle(client):
    if os.path.exists(os.path.join(ctrl, "block")):
        # Peer unreachable: the dial connects but the tunnel dies at once.
        try:
            client.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        client.close()
        return
    env = dict(os.environ)
    env["PTY_ROOT"] = srv_root
    env["PTY_ROOT_LEGACY_SILENT"] = "1"
    env.pop("PTY_SESSION", None)
    env.pop("PTY_SESSION_GENERATION", None)
    proc = subprocess.Popen(
        [pty_bin, "remote-serve", "--stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env,
    )
    pair = [client, proc]
    with lock:
        pairs.append(pair)
        write_active()

    def sock_to_child():
        try:
            while True:
                data = client.recv(65536)
                if not data:
                    break
                proc.stdin.write(data)
                proc.stdin.flush()
        except (OSError, ValueError):
            pass
        try:
            proc.stdin.close()
        except OSError:
            pass
        # sock.on("close", () => child.kill())
        try:
            proc.kill()
        except OSError:
            pass

    def child_to_sock():
        try:
            while True:
                data = proc.stdout.read1(65536)
                if not data:
                    break
                client.sendall(data)
        except (OSError, ValueError):
            pass
        # child.on("exit", () => sock.end())
        try:
            client.shutdown(socket.SHUT_WR)
        except OSError:
            pass

    a = threading.Thread(target=sock_to_child, daemon=True)
    b = threading.Thread(target=child_to_sock, daemon=True)
    a.start(); b.start()
    b.join()
    proc.wait()
    a.join(timeout=5)
    try:
        client.close()
    except OSError:
        pass
    remove(pair)

def watcher():
    drop = os.path.join(ctrl, "drop")
    while True:
        if os.path.exists(drop):
            with lock:
                live = list(pairs)
                pairs.clear()
                write_active()
            for pair in live:
                sever(pair)
            os.unlink(drop)
        time.sleep(0.02)

def on_term(*_):
    with lock:
        live = list(pairs)
        pairs.clear()
    for pair in live:
        sever(pair)
    try:
        os.unlink(listen)
    except OSError:
        pass
    os._exit(0)

signal.signal(signal.SIGTERM, on_term)
signal.signal(signal.SIGINT, on_term)
signal.signal(signal.SIGHUP, signal.SIG_IGN)

try:
    os.unlink(listen)
except OSError:
    pass
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(listen)
srv.listen(64)
write_active()
threading.Thread(target=watcher, daemon=True).start()
while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
"#;

/// A running bridge plus the stub `fabric` that points at it.
pub struct Bridge {
    child: Child,
    pub sock: PathBuf,
    pub ctrl: PathBuf,
    /// The "remote" registry the handlers read.
    pub srv_root: PathBuf,
}

impl Bridge {
    /// Start the bridge for `srv_root` and install `fabric` on the rig's PATH
    /// so `fabric dial <peer> pty-remote` prints the bridge socket.
    pub fn start(rig: &Rig, srv_root: &Path) -> Bridge {
        let script = rig.tmp().join("bridge.py");
        std::fs::write(&script, BRIDGE_PY).unwrap();
        // Everything lives under `<root>/fab/`: the socket must not sit in
        // the registry itself (a `*.sock` there is a session to `pty list`),
        // and the rig's teardown scans `*.pid` files recursively.
        let ctrl = rig.root().join("fab");
        std::fs::create_dir_all(&ctrl).unwrap();
        let sock = ctrl.join("s.sock");
        let pidfile = ctrl.join("bridge.pid");
        let mut cmd = Command::new("python3");
        cmd.arg(&script)
            .arg(&sock)
            .arg(pty_bin())
            .arg(srv_root)
            .arg(&ctrl)
            .arg(&pidfile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.env_clear();
        for (k, v) in rig.base_env() {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("spawn bridge.py");
        wait_until("bridge socket", || sock.exists());
        rig.stub_bin(
            "fabric",
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"dial\" ]; then printf '%s' \"{}\"; fi\n",
                sock.display()
            ),
        );
        Bridge {
            child,
            sock,
            ctrl,
            srv_root: srv_root.to_path_buf(),
        }
    }

    /// Live tunnels right now (`KillableFabricProxy.activeCount()`).
    pub fn active_count(&self) -> usize {
        std::fs::read_to_string(self.ctrl.join("active"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Sever every live tunnel; the listener keeps accepting (`.drop()`).
    pub fn drop_tunnels(&self) {
        let flag = self.ctrl.join("drop");
        std::fs::write(&flag, "1").unwrap();
        wait_until("bridge to sever the tunnels", || !flag.exists());
    }

    /// New dials fail at the transport level (`.block()`).
    pub fn block(&self) {
        std::fs::write(self.ctrl.join("block"), "1").unwrap();
    }

    /// New dials tunnel through again (`.unblock()`).
    pub fn unblock(&self) {
        let _ = std::fs::remove_file(self.ctrl.join("block"));
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let pid = self.child.id() as i32;
        kill_pid(pid, libc::SIGTERM);
        if !poll_for(Duration::from_secs(2), || self.child.try_wait().map(|s| s.is_some()).unwrap_or(true)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Start a session in the remote registry `srv_root` through the binary under
/// test; returns the `pty run -d` output.
pub fn remote_run(rig: &Rig, srv_root: &Path, args: &[&str]) -> Out {
    let mut all = vec!["run", "-d"];
    all.extend_from_slice(args);
    rig.pty_env(&[("PTY_ROOT", &srv_root.to_string_lossy())], &all)
}

/// Wait for `<id>.sock` in `srv_root`.
pub fn wait_remote_socket(srv_root: &Path, id: &str) {
    let sock = srv_root.join(format!("{id}.sock"));
    wait_until(&format!("{id} socket in the remote root"), || sock.exists());
}
