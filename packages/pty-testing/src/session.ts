/**
 * A terminal session you can type at and take pictures of.
 *
 * The session itself is hosted by the `pty` binary; this talks to its daemon
 * over the session socket. So what a test sees here is what any client sees.
 */

import { execFile, execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect, type Socket } from "node:net";

import { resolveKey } from "./keys.js";
import {
  MessageType,
  PacketReader,
  decodeExit,
  decodeGeometry,
  encodeAttach,
  encodeData,
  encodePeek,
  encodeResize,
} from "./protocol.js";

/** The environment variables a session must not inherit from the test run. */
const SCRUBBED = [
  "PTY_SESSION",
  "PTY_SESSION_GENERATION",
  "PTY_SESSION_DIR",
  "PTY_REAP_ON_EXIT",
  "NO_COLOR",
];

export interface SpawnOptions {
  rows?: number;
  cols?: number;
  cwd?: string;
  env?: Record<string, string>;
  /** The session id. A random one when absent. */
  name?: string;
}

export interface Screenshot {
  /** The visible lines, with trailing blank ones removed. */
  lines: string[];
  /** Those lines joined with newlines. */
  text: string;
  /** The same screen with its escape sequences intact. */
  ansi: string;
}

/** How long the waits allow when no timeout is given. */
export const DEFAULT_TIMEOUT_MS = 10_000;
const POLL_MS = 50;

let engineChecked = false;

/**
 * The `pty` to drive: `PTY_BIN`, else `pty` on PATH.
 *
 * The first call refuses a binary that is not the Rust one, because this
 * package is written against its behaviour. `PTY_TESTING_ALLOW_NODE=1` says
 * you meant it.
 */
export function ptyBin(): string {
  const bin = process.env.PTY_BIN || "pty";
  if (!engineChecked) {
    engineChecked = true;
    if (process.env.PTY_TESTING_ALLOW_NODE !== "1") {
      let version = "";
      try {
        version = execFileSync(bin, ["version"], { encoding: "utf8" }).trim();
      } catch (e) {
        throw new Error(`could not run "${bin} version": ${(e as Error).message}`);
      }
      if (!version.includes("-rust")) {
        throw new Error(
          `${bin} reports ${version}, which is not the Rust pty. ` +
            `Set PTY_BIN, or PTY_TESTING_ALLOW_NODE=1 if you meant this one.`,
        );
      }
    }
  }
  return bin;
}

function randomId(): string {
  const alphabet = "23456789abcdefghjkmnpqrstuvwxyz";
  let out = "";
  for (let i = 0; i < 8; i++) out += alphabet[Math.floor(Math.random() * alphabet.length)];
  return out;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class Session {
  #socket: Socket | null = null;
  #ansi = "";
  #rows: number;
  #cols: number;
  #exitCode: number | null = null;
  #exited = false;

  private constructor(
    readonly name: string,
    readonly root: string,
    private readonly bin: string,
    private readonly ownsSession: boolean,
    private readonly ownsRoot: boolean,
    rows: number,
    cols: number,
  ) {
    this.#rows = rows;
    this.#cols = cols;
  }

  get rows(): number {
    return this.#rows;
  }
  get cols(): number {
    return this.#cols;
  }
  get hasExited(): boolean {
    return this.#exited;
  }
  get exitCode(): number | null {
    return this.#exitCode;
  }

  /** Create a session and attach to it. */
  static async spawn(command: string, args: string[] = [], opts: SpawnOptions = {}): Promise<Session> {
    const bin = ptyBin();
    const rows = opts.rows ?? 24;
    const cols = opts.cols ?? 80;
    const name = opts.name ?? randomId();
    // Short, because a session socket path has to fit 104 bytes.
    const root = mkdtempSync(join(tmpdir(), "pt-"));

    const argv = ["run", "-d", "-e", "--no-display-name", "--id", name];
    if (opts.cwd) argv.push("--cwd", opts.cwd);
    argv.push("--rows", String(rows), "--cols", String(cols));
    for (const [k, v] of Object.entries(opts.env ?? {})) argv.push("--env", `${k}=${v}`);
    argv.push("--", command, ...args);

    const env: NodeJS.ProcessEnv = { ...process.env, PTY_ROOT: root };
    for (const key of SCRUBBED) delete env[key];
    await new Promise<void>((resolve, reject) => {
      execFile(bin, argv, { env }, (err, _out, stderr) => {
        if (err) reject(new Error(`pty run failed: ${String(stderr).trim() || err.message}`));
        else resolve();
      });
    });

    const session = new Session(name, root, bin, true, true, rows, cols);
    await session.attach();
    return session;
  }

  /** Attach to a session that already exists, without owning it. */
  static async connect(
    name: string,
    opts: { rows?: number; cols?: number; root: string },
  ): Promise<Session> {
    const session = new Session(
      name,
      opts.root,
      ptyBin(),
      false,
      false,
      opts.rows ?? 24,
      opts.cols ?? 80,
    );
    await session.attach();
    return session;
  }

  /** A second client on the same session, at its own size. */
  static async connectToExisting(
    other: Session,
    opts: { rows?: number; cols?: number } = {},
  ): Promise<Session> {
    return Session.connect(other.name, { ...opts, root: other.root });
  }

  /** Open the socket and join the session. Resolves on the first screen. */
  async attach(): Promise<void> {
    const path = join(this.root, `${this.name}.sock`);
    const deadline = Date.now() + 15_000;
    while (!existsSync(path)) {
      if (Date.now() > deadline) throw new Error(`session "${this.name}" never came up`);
      await sleep(20);
    }

    const socket = connect(path);
    this.#socket = socket;
    const reader = new PacketReader();
    let sawScreen = false;
    let onScreen: (() => void) | null = null;
    const screen = new Promise<void>((resolve) => {
      onScreen = resolve;
    });

    socket.on("data", (chunk: Buffer) => {
      let packets;
      try {
        packets = reader.feed(chunk);
      } catch {
        socket.destroy();
        return;
      }
      for (const packet of packets) {
        switch (packet.type) {
          case MessageType.Screen:
          case MessageType.Data:
            this.#ansi += packet.payload.toString("utf8");
            if (packet.type === MessageType.Screen && !sawScreen) {
              sawScreen = true;
              onScreen?.();
            }
            break;
          case MessageType.Geometry: {
            const { rows, cols } = decodeGeometry(packet.payload);
            this.#rows = rows;
            this.#cols = cols;
            break;
          }
          case MessageType.Exit:
            this.#exitCode = decodeExit(packet.payload);
            this.#exited = true;
            break;
        }
      }
    });
    socket.on("close", () => {
      onScreen?.();
    });
    socket.on("error", () => {
      onScreen?.();
    });

    await new Promise<void>((resolve, reject) => {
      socket.once("connect", () => resolve());
      socket.once("error", reject);
    });
    socket.write(encodeAttach(this.#rows, this.#cols));
    // The daemon replays the screen straight after ATTACH; wait for it so a
    // screenshot taken immediately is not empty.
    await Promise.race([screen, sleep(2000)]);
  }

  /** Drop the connection and open a new one, as a client that lost it would. */
  async reconnect(): Promise<void> {
    this.#socket?.destroy();
    this.#socket = null;
    this.#ansi = "";
    await sleep(100);
    await this.attach();
  }

  /** Send text as typed. */
  sendKeys(text: string): void {
    this.#socket?.write(encodeData(text));
  }

  /** Alias for {@link Session.sendKeys}. */
  type(text: string): void {
    this.sendKeys(text);
  }

  /** Send a named key: `ctrl+c`, `C-u`, `return`, `up`. */
  press(key: string): void {
    this.sendKeys(resolveKey(key));
  }

  /** Ask for a new size. The daemon gives every client the smallest asked for. */
  resize(rows: number, cols: number): void {
    this.#rows = rows;
    this.#cols = cols;
    this.#socket?.write(encodeResize(rows, cols));
  }

  /** The screen as it is now. */
  async screenshot(): Promise<Screenshot> {
    const plain = await this.#peek(true);
    const lines = plain.split("\n");
    while (lines.length > 0 && lines[lines.length - 1].trim() === "") lines.pop();
    return { lines, text: lines.join("\n"), ansi: this.#ansi };
  }

  /** Wait for `text` to appear, and return the screen it appeared on. */
  async waitForText(text: string, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<Screenshot> {
    return this.waitFor((s) => s.text.includes(text), timeoutMs, `text "${text}" to appear`);
  }

  /** Wait for `text` to go away. */
  async waitForAbsent(text: string, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<Screenshot> {
    return this.waitFor((s) => !s.text.includes(text), timeoutMs, `text "${text}" to go away`);
  }

  /** Wait until `predicate` is happy with the screen. */
  async waitFor(
    predicate: (screen: Screenshot) => boolean,
    timeoutMs = DEFAULT_TIMEOUT_MS,
    what = "the condition",
  ): Promise<Screenshot> {
    const deadline = Date.now() + timeoutMs;
    let last = await this.screenshot();
    for (;;) {
      if (predicate(last)) return last;
      if (Date.now() > deadline) break;
      await sleep(POLL_MS);
      last = await this.screenshot();
    }
    throw new Error(`Timed out after ${timeoutMs}ms waiting for ${what}. Screen was:\n${last.text}`);
  }

  /**
   * Stop the session if this handle created it, and clean up after it.
   * A handle that merely connected leaves the session running.
   */
  async close(): Promise<void> {
    this.#socket?.destroy();
    this.#socket = null;
    if (this.ownsSession) {
      const env: NodeJS.ProcessEnv = { ...process.env, PTY_ROOT: this.root };
      for (const verb of ["kill", "rm"]) {
        try {
          execFileSync(this.bin, [verb, this.name], { env, stdio: "ignore" });
        } catch {
          // Already gone is the wanted state.
        }
      }
    }
    if (this.ownsRoot) rmSync(this.root, { recursive: true, force: true });
  }

  /** A short-lived socket that reads the screen without joining. */
  async #peek(plain: boolean): Promise<string> {
    const path = join(this.root, `${this.name}.sock`);
    if (!existsSync(path)) {
      // The session ended; the last screen it saved is the best answer.
      const metaPath = join(this.root, `${this.name}.json`);
      if (existsSync(metaPath)) {
        try {
          const meta = JSON.parse(readFileSync(metaPath, "utf8"));
          if (Array.isArray(meta.lastLines)) return meta.lastLines.join("\n");
        } catch {
          // Fall through to an empty screen.
        }
      }
      return "";
    }
    return new Promise<string>((resolve) => {
      const socket = connect(path);
      const reader = new PacketReader();
      let out = "";
      const done = () => {
        socket.destroy();
        resolve(out);
      };
      socket.on("connect", () => socket.write(encodePeek(plain, false)));
      socket.on("data", (chunk: Buffer) => {
        let packets;
        try {
          packets = reader.feed(chunk);
        } catch {
          done();
          return;
        }
        for (const packet of packets) {
          if (packet.type === MessageType.Screen || packet.type === MessageType.Data) {
            out += packet.payload.toString("utf8");
          }
        }
      });
      socket.on("close", done);
      socket.on("error", done);
      setTimeout(done, 2000);
    });
  }
}
