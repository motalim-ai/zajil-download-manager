import { describe, expect, it } from "vitest";
// @ts-expect-error: a plain JavaScript script, without types.
import { allowed, SHIPPED } from "../scripts/check-licenses.mjs";

describe("the npm license check", () => {
  it("reads SPDX expressions with their parentheses, AND, OR and WITH", () => {
    expect(allowed("MIT", SHIPPED)).toBe(true);
    expect(allowed("(MIT OR Apache-2.0)", SHIPPED)).toBe(true);
    expect(allowed("MIT AND ISC", SHIPPED)).toBe(true);
    expect(allowed("Apache-2.0 WITH LLVM-exception", SHIPPED)).toBe(true);
    expect(allowed("GPL-2.0-only OR MIT", SHIPPED)).toBe(true);
    // Review of #45: this passed when parentheses were dropped.
    expect(allowed("(MIT OR GPL-2.0-only) AND SSPL-1.0", SHIPPED)).toBe(false);
    expect(allowed("MIT AND SSPL-1.0", SHIPPED)).toBe(false);
    expect(allowed("SSPL-1.0", SHIPPED)).toBe(false);
    expect(allowed("MPL-2.0", SHIPPED)).toBe(false);
    // An exception that restricts the license (review of #45).
    expect(allowed("Apache-2.0 WITH Commons-Clause", SHIPPED)).toBe(false);
  });
  it("refuses what it does not understand", () => {
    for (const odd of ["", "UNKNOWN", "(MIT", "MIT OR", "MIT AND (ISC", "MIT ISC", "SEE LICENSE IN LICENSE", "MIT WITH OR", "MIT WITH (", "MIT WITH"]) {
      expect(allowed(odd, SHIPPED), odd).toBe(false);
    }
  });
});
