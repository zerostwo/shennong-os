import { describe, expect, it } from "vitest";
import { settingsHash, settingsSectionFromHash } from "./settings-route";

describe("settings hash routing", () => {
  it("uses ChatGPT-style settings fragments", () => {
    expect(settingsHash("general")).toBe("#settings/General");
    expect(settingsHash("models")).toBe("#settings/Models");
    expect(settingsHash("account")).toBe("#settings/Account");
    expect(settingsHash("personalization")).toBe("#settings/Personalization");
    expect(settingsHash("keyboard")).toBe("#settings/Keyboard");
    expect(settingsHash("memory")).toBe("#settings/Memory");
  });

  it("parses settings routes case-insensitively and rejects unrelated hashes", () => {
    expect(settingsSectionFromHash("#settings/Memory")).toBe("memory");
    expect(settingsSectionFromHash("#SETTINGS/account")).toBe("account");
    expect(settingsSectionFromHash("#settings/Skills")).toBeNull();
    expect(settingsSectionFromHash("#project/Account")).toBeNull();
  });
});
