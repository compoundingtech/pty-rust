/**
 * Key names to the bytes a terminal sends.
 *
 * `ctrl+u`, `ctrl-u`, `ctrl_u` and `C-u` all mean the same key. The
 * modifiers are ctrl, alt and shift; the keys are a to z and the names
 * below.
 */

const KEY_MAP: Record<string, string> = {
  return: "\r",
  enter: "\r",
  tab: "\t",
  escape: "\x1b",
  esc: "\x1b",
  space: " ",
  backspace: "\x7f",
  delete: "\x1b[3~",
  up: "\x1b[A",
  down: "\x1b[B",
  right: "\x1b[C",
  left: "\x1b[D",
  home: "\x1b[H",
  end: "\x1b[F",
  pageup: "\x1b[5~",
  pagedown: "\x1b[6~",
};

const MODIFIERS = new Set(["ctrl", "alt", "shift"]);
const SEPARATORS = /[+_-]/;
const NAMED_KEYS = Object.keys(KEY_MAP).sort().join(", ");

const HELP =
  `Use ctrl+u, ctrl-u, ctrl_u, or C-u; supported modifiers are ctrl, alt, and shift; ` +
  `supported keys are a-z, ${NAMED_KEYS}.`;

/** Keycodes for the control keys under a modifier (CSI u). */
const CSI_U: Record<string, number> = {
  return: 13,
  enter: 13,
  tab: 9,
  escape: 27,
  esc: 27,
  space: 32,
  backspace: 127,
};

/** 1 + shift(1) + alt(2) + ctrl(4), the xterm modifier parameter. */
function modifierParam(mods: Set<string>): number {
  return (
    1 + (mods.has("shift") ? 1 : 0) + (mods.has("alt") ? 2 : 0) + (mods.has("ctrl") ? 4 : 0)
  );
}

/**
 * `C-u` is the readline and tmux spelling of ctrl+u. The one-letter alias is
 * scoped to a leading `C-`, so `C+u` keeps no surprise meaning.
 */
function normalizeModifier(mod: string, index: number, spec: string): string {
  if (mod === "c" && index === 0 && /^c-/i.test(spec)) return "ctrl";
  return mod;
}

function isSupportedBase(base: string): boolean {
  return KEY_MAP[base] !== undefined || (base.length === 1 && base >= "a" && base <= "z");
}

/** The bytes for a key spec. Throws with the help text on a bad one. */
export function resolveKey(spec: string): string {
  const normalized = spec.toLowerCase();
  const hasSeparator = SEPARATORS.test(normalized);
  const rawParts = hasSeparator ? normalized.split(SEPARATORS) : [normalized];
  const rawBase = rawParts.at(-1) ?? "";
  const rawMods = rawParts.slice(0, -1).map((m, i) => normalizeModifier(m, i, spec));

  // A name with a separator in it could be read two ways. Refuse rather than
  // silently pick one.
  const isValidChord =
    rawBase !== "" &&
    rawMods.length > 0 &&
    rawMods.every((m) => m !== "" && MODIFIERS.has(m)) &&
    isSupportedBase(rawBase);
  if (hasSeparator && KEY_MAP[normalized] !== undefined && isValidChord) {
    throw new Error(
      `Ambiguous key spec "${spec}": it is both a named key and a modifier chord. ${HELP}`,
    );
  }
  if (KEY_MAP[normalized] !== undefined && !isValidChord) return KEY_MAP[normalized];

  const parts = [...rawParts];
  const base = parts.pop() as string;
  if (base === "" || parts.some((p) => p === "")) {
    throw new Error(`Incomplete key spec "${spec}". ${HELP}`);
  }
  const mods = new Set(parts.map((m, i) => normalizeModifier(m, i, spec)));
  for (const mod of mods) {
    if (!MODIFIERS.has(mod)) {
      throw new Error(`Unknown modifier: "${mod}" in key spec "${spec}". ${HELP}`);
    }
  }

  const isLetter = base.length === 1 && base >= "a" && base <= "z";
  const mapped = KEY_MAP[base];
  if (mapped === undefined && !isLetter) {
    throw new Error(`Unknown key: "${base}" in key spec "${spec}". ${HELP}`);
  }

  if (isLetter) {
    let result = base;
    if (mods.has("shift")) result = result.toUpperCase();
    if (mods.has("ctrl")) result = String.fromCharCode(result.toLowerCase().charCodeAt(0) - 96);
    if (mods.has("alt")) result = "\x1b" + result;
    return result;
  }

  if (mods.size === 0) return mapped;

  const param = modifierParam(mods);
  // shift+tab has its own old sequence.
  if (base === "tab" && param === 2) return "\x1b[Z";

  const tilde = /^\x1b\[(\d+)~$/.exec(mapped);
  if (tilde) return `\x1b[${tilde[1]};${param}~`;
  const letter = /^\x1b\[([A-Z])$/.exec(mapped);
  if (letter) return `\x1b[1;${param}${letter[1]}`;
  const code = CSI_U[base];
  if (code !== undefined) return `\x1b[${code};${param}u`;
  return mapped;
}
