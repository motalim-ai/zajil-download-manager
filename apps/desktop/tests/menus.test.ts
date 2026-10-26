import { describe, expect, it } from "vitest";
import { accelerator, commandFor, keyOf, label, KEYS, type Command } from "../src/lib/commands";
import { MENU_BAR, ROW_MENU, accessLetter, commandKey } from "../src/lib/menus";
import { COLUMNS, DEFAULT, MAX_WIDTH, MIN_WIDTH, clean, load, move, save, toggle, visible } from "../src/lib/columns";
import { LANGS, makeBundle } from "../src/lib/i18n.svelte";

// M5.5 (the design of #83): the menus' commands, their shortcuts, and the
// table's columns as the user leaves them.
const key = (init: Partial<KeyboardEvent>) =>
  ({ key: "", code: "", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...init }) as KeyboardEvent;

const commands = (): Command[] => {
  const all = [...MENU_BAR.flatMap((m) => m.items), ...ROW_MENU];
  return all.flatMap((i) => (typeof i === "object" && "command" in i ? [i.command] : []));
};

describe("the menus", () => {
  it("name every command, menu and system item in both languages", () => {
    const keys = [
      ...commands().map(commandKey),
      ...MENU_BAR.map((m) => m.key),
      "menu-bar",
      "cmd-move-to-queue",
      "cmd-columns",
      ...["hide", "hide-others", "show-all", "undo", "redo", "cut", "copy", "paste", "window", "minimize", "zoom"].map((k) => `sys-${k}`),
    ];
    for (const lang of LANGS) {
      const bundle = makeBundle(lang);
      for (const k of keys) expect(bundle.hasMessage(k), `${lang}: ${k}`).toBe(true);
    }
  });

  it("open by their names' first letters", () => {
    expect(accessLetter("ملف")).toBe("م");
    expect(accessLetter("Edit")).toBe("E");
  });
});

describe("the shortcuts", () => {
  it("are the design's on each system", () => {
    expect(label(keyOf("pause-all", false)!, false)).toBe("Ctrl+Shift+P");
    expect(label(keyOf("pause-all", true)!, true)).toBe("⌘⇧P");
    expect(label(keyOf("delete", false)!, false)).toBe("Del");
    expect(label(keyOf("delete", true)!, true)).toBe("⌘⌫");
    expect(label(keyOf("run-queue", false)!, false)).toBe("F5");
    expect(label(keyOf("run-queue", true)!, true)).toBe("⌘⇧R");
    expect(label(keyOf("properties", false)!, false)).toBe("Alt+Enter");
    expect(label(keyOf("properties", true)!, true)).toBe("⌘I");
    expect(accelerator(keyOf("pause-all", true)!)).toBe("CmdOrCtrl+Shift+P");
  });

  it("are each one command's, on each system", () => {
    for (const mac of [false, true]) {
      const seen = Object.values(KEYS).map((k) => label(mac ? k!.mac : k!.other, mac));
      expect(new Set(seen).size).toBe(seen.length);
    }
  });

  it("find their command, with exactly its modifiers", () => {
    expect(commandFor(key({ key: "p", ctrlKey: true }), false)).toBe("pause");
    expect(commandFor(key({ key: "P", ctrlKey: true, shiftKey: true }), false)).toBe("pause-all");
    expect(commandFor(key({ key: "p", metaKey: true }), false)).toBeUndefined();
    expect(commandFor(key({ key: "p", metaKey: true }), true)).toBe("pause");
    expect(commandFor(key({ key: "F5" }), false)).toBe("run-queue");
    expect(commandFor(key({ key: "F5", shiftKey: true }), false)).toBe("halt-queue");
    expect(commandFor(key({ key: "Delete" }), false)).toBe("delete");
    expect(commandFor(key({ key: "Backspace", metaKey: true }), true)).toBe("delete");
    expect(commandFor(key({ key: "Backspace" }), true)).toBeUndefined();
    expect(commandFor(key({ key: "Enter", altKey: true }), false)).toBe("properties");
    // The Arabic layout: «ح» is on P.
    expect(commandFor(key({ key: "ح", code: "KeyP", ctrlKey: true }), false)).toBe("pause");
    expect(commandFor(key({ key: "x" }), false)).toBeUndefined();
  });
});

describe("the columns", () => {
  const memory = () => {
    const kept = new Map<string, string>();
    return { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  };

  it("come back as they were left", () => {
    const storage = memory();
    const changed = toggle(move({ ...DEFAULT, widths: { size: 140 } }, "speed", "name"), "left");
    save(changed, storage);
    expect(load(storage)).toEqual(changed);
    expect(visible(load(storage)).map((c) => c.id)).toEqual(["speed", "name", "size", "status", "last", "desc", "queue"]);
    expect(visible(load(storage)).find((c) => c.id === "size")?.width).toBe(140);
  });

  it("are the defaults when nothing, or nothing readable, was kept", () => {
    expect(load(memory())).toEqual(DEFAULT);
    expect(load({ getItem: () => "{not json" })).toEqual(DEFAULT);
    expect(
      load({
        getItem: () => {
          throw new Error("blocked");
        },
      }),
    ).toEqual(DEFAULT);
    expect(() => save(DEFAULT, { setItem: () => { throw new Error("full"); } })).not.toThrow();
  });

  it("drop what does not belong, and keep the name shown", () => {
    const kept = clean({ order: ["desc", "nope", "desc", 3], hidden: ["name", "size", "size"], widths: { size: 9999, left: 1, name: 300, x: 5 } });
    expect(kept.order).toEqual(["desc", ...COLUMNS.map((c) => c.id).filter((id) => id !== "desc")]);
    expect(kept.hidden).toEqual(["size"]);
    expect(kept.widths).toEqual({ size: MAX_WIDTH, left: MIN_WIDTH });
    expect(toggle(DEFAULT, "name")).toBe(DEFAULT);
  });
});
