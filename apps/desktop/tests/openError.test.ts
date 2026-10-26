import { describe, expect, it } from "vitest";
import { openError } from "../src/lib/openError";

const t = (key: string) => `«${key}»`;

describe("opening a download that is not there any more (#98)", () => {
  it("says it in the user's language", () => {
    expect(openError("no-file", t)).toBe("«open-no-file»");
    expect(openError("no-folder", t)).toBe("«open-no-folder»");
  });

  it("keeps any other error as it is", () => {
    expect(openError("no download 7", t)).toBe("no download 7");
  });
});
