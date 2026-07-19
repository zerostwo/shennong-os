import { describe, expect, it } from "vitest";
import { formatDateTime } from "./format-date";

describe("formatDateTime", () => {
  it("formats ISO timestamps for people instead of exposing the wire value", () => {
    const value = formatDateTime("2026-07-19T08:30:00.000Z");
    expect(value).not.toContain("T08:30:00.000Z");
    expect(value).toContain("2026");
  });

  it("keeps invalid service values visible and falls back for missing values", () => {
    expect(formatDateTime("not-a-date")).toBe("not-a-date");
    expect(formatDateTime(undefined)).toBe("Not available");
  });
});
