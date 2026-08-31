import { afterAll, describe, expect, it } from "vitest";

import { Session } from "../src/index.js";

const open = (command: string, args: string[] = [], rows = 24, cols = 80) =>
  Session.spawn(command, args, { rows, cols });

describe("a session", () => {
  const opened: Session[] = [];
  const track = async (s: Promise<Session>) => {
    const session = await s;
    opened.push(session);
    return session;
  };
  afterAll(async () => {
    await Promise.all(opened.map((s) => s.close()));
  });

  it("shows what the session printed before this client arrived", async () => {
    const s = await track(open("sh", ["-c", "printf 'BEFORE-ATTACH\\n'; exec cat"]));
    const screen = await s.waitForText("BEFORE-ATTACH");
    expect(screen.text).toContain("BEFORE-ATTACH");
  });

  it("sends what you type and shows the answer", async () => {
    const s = await track(
      open("sh", ["-c", "stty -echo; while read line; do echo \"got:$line\"; done"]),
    );
    await new Promise((r) => setTimeout(r, 200));
    s.type("hello\r");
    const screen = await s.waitForText("got:hello");
    expect(screen.text).toContain("got:hello");
  });

  it("sends a named key", async () => {
    const s = await track(
      open("sh", ["-c", "stty -echo; while read line; do echo \"got:$line\"; done"]),
    );
    await new Promise((r) => setTimeout(r, 200));
    s.type("pressed");
    s.press("return");
    await s.waitForText("got:pressed");
  });

  it("rebuilds the screen from the daemon after a reconnect", async () => {
    const s = await track(open("sh", ["-c", "printf 'STAYS-ON-SCREEN\\n'; exec cat"]));
    await s.waitForText("STAYS-ON-SCREEN");
    await s.reconnect();
    const screen = await s.waitForText("STAYS-ON-SCREEN");
    expect(screen.text).toContain("STAYS-ON-SCREEN");
  });

  it("lets a second client watch, and the smaller size wins", async () => {
    const first = await track(open("sh", ["-c", "printf 'SHARED-LINE\\n'; exec cat"]));
    await first.waitForText("SHARED-LINE");

    const second = await Session.connectToExisting(first, { rows: 10, cols: 40 });
    try {
      await second.waitForText("SHARED-LINE");
      await second.waitFor(() => second.cols === 40, 5000, "the smaller width");
      expect(second.rows).toBe(10);
      expect(second.cols).toBe(40);
    } finally {
      // A client that did not create the session leaves it running.
      await second.close();
    }
    await first.waitForText("SHARED-LINE");
  });

  it("reports the exit status", async () => {
    const s = await track(open("sh", ["-c", "exit 7"]));
    await s.waitFor(() => s.hasExited, 8000, "the session to end");
    expect(s.exitCode).toBe(7);
  });

  it("says what it was waiting for when it times out", async () => {
    const s = await track(open("cat"));
    await expect(s.waitForText("NEVER-APPEARS", 300)).rejects.toThrow(/NEVER-APPEARS/);
  });
});
