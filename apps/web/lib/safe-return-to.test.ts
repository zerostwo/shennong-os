import { describe, expect, it } from "vitest";

import { safeInternalReturnTo } from "./safe-return-to";

describe("safeInternalReturnTo", () => {
  it.each([
    "/projects",
    "/projects/project-1?tab=graph#focus",
    "/resources?query=single%20cell",
  ])("accepts internal application location %s", (value) => {
    expect(safeInternalReturnTo(value)).toBe(value);
  });

  it.each([
    "https://attacker.example/",
    "//attacker.example/",
    "/\\attacker.example/",
    "/%5cattacker.example/",
    "/%2fattacker.example/",
    "/%252f%252fattacker.example/",
    "/%2525252525252525252f%2525252525252525252fattacker.example/",
    "/projects\nnext",
    "/projects%0d%0aLocation%3A%20https%3A%2F%2Fattacker.example",
    "/projects%00",
    "/projects%ZZ",
  ])("rejects unsafe return location %s", (value) => {
    expect(safeInternalReturnTo(value)).toBe("/projects");
  });

  it("uses the caller-provided fallback for absent input", () => {
    expect(safeInternalReturnTo(null, "/")).toBe("/");
  });
});
