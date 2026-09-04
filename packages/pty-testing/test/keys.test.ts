import { describe, expect, it } from "vitest";

import { resolveKey } from "../src/keys.js";

describe("key specs", () => {
  it("accepts every spelling of a control chord", () => {
    for (const spec of ["ctrl+u", "ctrl-u", "ctrl_u", "C-u"]) {
      expect(resolveKey(spec)).toBe("\x15");
    }
  });

  it("resolves named keys", () => {
    expect(resolveKey("return")).toBe("\r");
    expect(resolveKey("up")).toBe("\x1b[A");
    expect(resolveKey("escape")).toBe("\x1b");
  });

  it("gives shift+tab its own sequence", () => {
    expect(resolveKey("shift+tab")).toBe("\x1b[Z");
  });

  it("says what is wrong with a spec it cannot read", () => {
    expect(() => resolveKey("ctrl-")).toThrow(/Incomplete key spec/);
    expect(() => resolveKey("bogus")).toThrow(/Unknown key/);
    expect(() => resolveKey("x+u")).toThrow(/Unknown modifier/);
  });
});
