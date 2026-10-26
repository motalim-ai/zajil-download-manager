// The window's commands (M5.5, the design of #83): one list for the menu
// bar, the right-click menu, the toolbar and the keyboard. Each shortcut is
// the one in the design README's table, with macOS's own where it differs.
import { onMac } from "./keys";

export type Command =
  | "add"
  | "video"
  | "batch"
  | "batch-clipboard"
  | "grab"
  | "import"
  | "export"
  | "quit"
  | "select-all"
  | "find"
  | "copy-link"
  | "delete-done"
  | "resume"
  | "pause"
  | "pause-all"
  | "delete"
  | "run-queue"
  | "halt-queue"
  | "schedule"
  | "settings"
  | "categories"
  | "columns"
  | "open"
  | "open-folder"
  | "progress"
  | "refresh-link"
  | "properties"
  | "remove-from-queue"
  | "about";

/** A key, as `KeyboardEvent.key` names it (one letter in lower case), and
 * the modifiers it needs. `mod` is ⌘ on macOS and Ctrl elsewhere. */
export type Key = { key: string; mod?: boolean; shift?: boolean; alt?: boolean };

/** The shortcuts: Windows and Linux, then macOS. */
export const KEYS: Partial<Record<Command, { other: Key; mac: Key }>> = {
  add: { other: { key: "n", mod: true }, mac: { key: "n", mod: true } },
  quit: { other: { key: "q", mod: true }, mac: { key: "q", mod: true } },
  resume: { other: { key: "r", mod: true }, mac: { key: "r", mod: true } },
  pause: { other: { key: "p", mod: true }, mac: { key: "p", mod: true } },
  "pause-all": { other: { key: "p", mod: true, shift: true }, mac: { key: "p", mod: true, shift: true } },
  delete: { other: { key: "Delete" }, mac: { key: "Backspace", mod: true } },
  "delete-done": { other: { key: "Delete", mod: true, shift: true }, mac: { key: "Backspace", mod: true, shift: true } },
  settings: { other: { key: ",", mod: true }, mac: { key: ",", mod: true } },
  schedule: { other: { key: "j", mod: true }, mac: { key: "j", mod: true } },
  // F5 is a media key on macOS: ⌘⇧R (run) and ⌘⇧H (halt) there.
  "run-queue": { other: { key: "F5" }, mac: { key: "r", mod: true, shift: true } },
  "halt-queue": { other: { key: "F5", shift: true }, mac: { key: "h", mod: true, shift: true } },
  find: { other: { key: "f", mod: true }, mac: { key: "f", mod: true } },
  "select-all": { other: { key: "a", mod: true }, mac: { key: "a", mod: true } },
  // Enter renames in Finder: ⌘O opens there.
  open: { other: { key: "Enter" }, mac: { key: "o", mod: true } },
  "open-folder": { other: { key: "o", mod: true, shift: true }, mac: { key: "o", mod: true, shift: true } },
  "copy-link": { other: { key: "c", mod: true }, mac: { key: "c", mod: true } },
  // ⌘I is «Get Info» on macOS.
  properties: { other: { key: "Enter", alt: true }, mac: { key: "i", mod: true } },
};

/** The shortcut of `command` on this system, if it has one. */
export function keyOf(command: Command, mac = onMac()): Key | undefined {
  const keys = KEYS[command];
  return keys ? (mac ? keys.mac : keys.other) : undefined;
}

const NAMES: Record<string, { mac: string; other: string }> = {
  Delete: { mac: "⌦", other: "Del" },
  Backspace: { mac: "⌫", other: "Backspace" },
  Enter: { mac: "↩", other: "Enter" },
  ",": { mac: ",", other: "," },
};

/** How a shortcut is written: `Ctrl+Shift+P` on Windows and Linux, `⌘⇧P` on
 * macOS. */
export function label(key: Key, mac = onMac()): string {
  const name = NAMES[key.key]?.[mac ? "mac" : "other"] ?? (key.key.length === 1 ? key.key.toUpperCase() : key.key);
  if (mac) return `${key.mod ? "⌘" : ""}${key.alt ? "⌥" : ""}${key.shift ? "⇧" : ""}${name}`;
  return [key.mod && "Ctrl", key.alt && "Alt", key.shift && "Shift", name].filter(Boolean).join("+");
}

/** The shortcut of `command`, as written on this system, or "". */
export function shortcutOf(command: Command, mac = onMac()): string {
  const key = keyOf(command, mac);
  return key ? label(key, mac) : "";
}

/** As Tauri's menus write an accelerator (`CmdOrCtrl+Shift+P`), for the
 * macOS menu bar. */
export function accelerator(key: Key): string {
  const name = key.key.length === 1 ? key.key.toUpperCase() : key.key;
  return [key.mod && "CmdOrCtrl", key.alt && "Alt", key.shift && "Shift", name].filter(Boolean).join("+");
}

/** The place on the keyboard (`KeyboardEvent.code`) of a key's character
 * on a US layout: a letter, a digit, or the punctuation a shortcut uses. */
const PUNCTUATION: Record<string, string> = {
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};
function placeOf(key: string): string | undefined {
  if (/^[a-z]$/.test(key)) return `Key${key.toUpperCase()}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  return PUNCTUATION[key];
}

/** Whether `event` is `key`: by the character, or by the key's place when
 * the layout gives no Latin letter (Arabic: its comma key gives «و», #98),
 * with exactly its modifiers. */
export function matches(event: KeyboardEvent, key: Key, mac = onMac()): boolean {
  const mod = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  const none = !event.ctrlKey && !event.metaKey;
  if ((key.mod ? !mod : !none) || Boolean(key.shift) !== event.shiftKey || Boolean(key.alt) !== event.altKey) return false;
  if (key.key.length > 1) return event.key === key.key;
  const latin = /^[\x21-\x7e]$/.test(event.key);
  const place = placeOf(key.key);
  return event.key.toLowerCase() === key.key || (!latin && place !== undefined && event.code === place);
}

/** The command `event` is the shortcut of, if any. */
export function commandFor(event: KeyboardEvent, mac = onMac()): Command | undefined {
  for (const [command, keys] of Object.entries(KEYS)) {
    if (keys && matches(event, mac ? keys.mac : keys.other, mac)) return command as Command;
  }
  return undefined;
}
