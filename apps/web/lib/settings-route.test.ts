import { describe, expect, it } from "vitest";
import { settingsHash, settingsSectionFromHash } from "./settings-route";

describe("settings hash routing", () => {
  it("uses ChatGPT-style settings fragments", () => {
    expect(settingsHash("general")).toBe("#settings/General");
    expect(settingsHash("models")).toBe("#settings/Models");
    expect(settingsHash("skills")).toBe("#settings/Skills");
    expect(settingsHash("memory")).toBe("#settings/Memory");
  });

  it("parses settings routes case-insensitively and rejects unrelated hashes", () => {
    expect(settingsSectionFromHash("#settings/Memory")).toBe("memory");
    expect(settingsSectionFromHash("#SETTINGS/skills")).toBe("skills");
    expect(settingsSectionFromHash("#settings/Account")).toBeNull();
    expect(settingsSectionFromHash("#project/Account")).toBeNull();
  });
});
