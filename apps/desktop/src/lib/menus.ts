// The menu bar and the right-click menu (M5.5, the design of #83): what is
// in them, for the page's own bar (Windows and Linux) and the system's
// (macOS, `menu.rs`).
import type { Command } from "./commands";

/** An item: a command, a separator, or (right-click only) a submenu of
 * queues. */
export type Item = { command: Command; danger?: boolean } | "-" | { queues: true };

/** A menu: its name's key in the translations, and its items. */
export type Menu = { key: string; edit?: boolean; items: Item[] };

export const MENU_BAR: Menu[] = [
  { key: "menu-file", items: [{ command: "add" }, "-", { command: "import" }, { command: "export" }, "-", { command: "quit" }] },
  {
    key: "menu-edit",
    edit: true,
    items: [{ command: "copy-link" }, { command: "select-all" }, { command: "find" }, "-", { command: "delete-done" }],
  },
  {
    key: "menu-downloads",
    items: [
      { command: "resume" },
      { command: "pause" },
      { command: "pause-all" },
      "-",
      { command: "delete", danger: true },
      "-",
      { command: "run-queue" },
      { command: "halt-queue" },
      { command: "schedule" },
    ],
  },
  { key: "menu-view", items: [{ command: "columns" }, { command: "categories" }, "-", { command: "settings" }] },
  { key: "menu-help", items: [{ command: "about" }] },
];

/** The right-click menu on the selected downloads, as in the design. */
export const ROW_MENU: Item[] = [
  { command: "open" },
  { command: "open-folder" },
  "-",
  { command: "resume" },
  { command: "pause" },
  { command: "progress" },
  "-",
  { queues: true },
  { command: "remove-from-queue" },
  "-",
  { command: "copy-link" },
  { command: "refresh-link" },
  "-",
  { command: "delete", danger: true },
  { command: "properties" },
];

/** A command's name in the translations. */
export const commandKey = (command: Command): string => `cmd-${command}`;

/** A menu's access letter (Alt with it opens the menu): its name's first
 * letter, underlined. */
export const accessLetter = (name: string): string => [...name][0] ?? "";
