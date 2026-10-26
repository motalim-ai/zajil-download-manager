import { describe, expect, it } from "vitest";
import { DEFAULT, MENU_COLUMN_WIDTH, NAME_MIN_WIDTH, minTableWidth, toggle, visible } from "../src/lib/columns";

// The table's least width (#184): the name keeps room to be read, and the
// table scrolls sideways when the window is narrower.
describe("the table's least width", () => {
  it("is the fixed columns, the columns menu and the name's least width", () => {
    const shown = visible(DEFAULT);
    const fixed = shown.reduce((sum, c) => sum + (c.width ?? 0), 0);
    expect(minTableWidth(shown)).toBe(fixed + MENU_COLUMN_WIDTH + NAME_MIN_WIDTH);
    // At 1024 px, with the sidebar, the default columns no longer leave
    // the name less than its least width: the table is wider instead.
    expect(minTableWidth(shown) - fixed - MENU_COLUMN_WIDTH).toBe(NAME_MIN_WIDTH);
  });

  it("follows the columns shown and their widths", () => {
    const fewer = visible(toggle(toggle(DEFAULT, "queue"), "desc"));
    expect(minTableWidth(fewer)).toBe(minTableWidth(visible(DEFAULT)) - 120 - 88);
    const wider = visible({ ...DEFAULT, widths: { size: 300 } });
    expect(minTableWidth(wider)).toBe(minTableWidth(visible(DEFAULT)) + 200);
  });
});
