import { describe, expect, it } from "vitest";
import { isMod, shortcut } from "../src/lib/keys";
import { errorCode } from "../src/lib/components/DownloadTable.svelte";

// M5.5: shortcuts as the design README writes them, Cmd on macOS where the
// others use Ctrl; and the status column's «Error 403».
describe("shortcuts", () => {
  it("are shown as each system writes them", () => {
    expect(shortcut("mod+F", false)).toBe("Ctrl+F");
    expect(shortcut("mod+shift+P", false)).toBe("Ctrl+Shift+P");
    expect(shortcut("mod+F", true)).toBe("⌘F");
    expect(shortcut("mod+shift+Backspace", true)).toBe("⌘⇧⌫");
  });

  it("take Cmd on macOS and Ctrl elsewhere, with no other modifier", () => {
    // Node has no KeyboardEvent: what isMod reads is enough.
    const key = (init: Partial<KeyboardEvent>) =>
      ({ ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...init }) as KeyboardEvent;
    expect(isMod(key({ key: "f", ctrlKey: true }), "f", false)).toBe(true);
    expect(isMod(key({ key: "F", ctrlKey: true }), "f", false)).toBe(true);
    expect(isMod(key({ key: "f", metaKey: true }), "f", false)).toBe(false);
    expect(isMod(key({ key: "f", metaKey: true }), "f", true)).toBe(true);
    expect(isMod(key({ key: "f", ctrlKey: true }), "f", true)).toBe(false);
    expect(isMod(key({ key: "f", ctrlKey: true, shiftKey: true }), "f", false)).toBe(false);
    expect(isMod(key({ key: "a", ctrlKey: true }), "f", false)).toBe(false);
    // The Arabic layout: the key is «ب», its place is still F.
    expect(isMod(key({ key: "ب", code: "KeyF", ctrlKey: true }), "f", false)).toBe(true);
    expect(isMod(key({ key: "ش", code: "KeyA", metaKey: true }), "a", true)).toBe(true);
  });
});

describe("the status of a failed download", () => {
  it("names the server's code when there is one", () => {
    expect(errorCode("Zajil cannot download it: the server answered 403 Forbidden")).toBe("403");
    expect(errorCode("the server answered 404 Not Found")).toBe("404");
    expect(errorCode("connection reset")).toBeNull();
    expect(errorCode(null)).toBeNull();
  });
});
