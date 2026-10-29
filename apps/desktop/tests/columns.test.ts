import { describe, expect, it } from "vitest";
import { DEFAULT, MENU_COLUMN_WIDTH, NAME_MIN_WIDTH, minTableWidth, toggle, visible } from "../src/lib/columns";

// The table's least width (#184): the name keeps room to be read, and the
// table scrolls sideways when the window is narrower.
describe("the table's least width", () => {
  it("is the fixed columns, the columns menu and the name's least width", () => {
    // 748 for the seven fixed columns, 34 for the menu, 220 for the name:
    // a change to the default widths shows here.
    expect(minTableWidth(visible(DEFAULT))).toBe(1002);
    expect(MENU_COLUMN_WIDTH + NAME_MIN_WIDTH).toBe(254);
    // With the 228 px sidebar, the default table fits a window of 1230 px;
    // narrower, it scrolls sideways (the columns button stays in view).
    expect(minTableWidth(visible(DEFAULT)) + 228).toBe(1230);
  });

  it("follows the columns shown and their widths", () => {
    const fewer = visible(toggle(toggle(DEFAULT, "queue"), "desc"));
    expect(minTableWidth(fewer)).toBe(minTableWidth(visible(DEFAULT)) - 108 - 88);
    const wider = visible({ ...DEFAULT, widths: { size: 300 } });
    expect(minTableWidth(wider)).toBe(minTableWidth(visible(DEFAULT)) + 200);
  });
});
