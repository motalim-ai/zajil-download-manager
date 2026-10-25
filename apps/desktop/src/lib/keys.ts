// Keyboard shortcuts (M5.5, the table in the design README of #83): Cmd on
// macOS where Windows and Linux use Ctrl.

/** Whether this is macOS. */
export const onMac = (): boolean => /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? "");

/** How a shortcut is shown: `mod+F` is ⌘F on macOS and Ctrl+F elsewhere;
 * `shift` is ⇧ on macOS. */
export function shortcut(keys: string, mac = onMac()): string {
  const parts = keys.split("+");
  if (mac) {
    return parts
      .map((p) => ({ mod: "⌘", shift: "⇧", alt: "⌥", Backspace: "⌫" })[p] ?? p)
      .join("");
  }
  return parts.map((p) => ({ mod: "Ctrl", shift: "Shift", alt: "Alt" })[p] ?? p).join("+");
}

/** Whether `event` is the platform's modifier (Cmd on macOS, Ctrl elsewhere)
 * with `key`, and no other modifier. */
export function isMod(event: KeyboardEvent, key: string, mac = onMac()): boolean {
  const mod = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  // By the key's place too: with the Arabic layout, Ctrl+F gives «ب» as
  // `key`, but `code` is still KeyF (review of #96).
  const same = event.key.toLowerCase() === key.toLowerCase() || event.code === `Key${key.toUpperCase()}`;
  return mod && !event.altKey && !event.shiftKey && same;
}
