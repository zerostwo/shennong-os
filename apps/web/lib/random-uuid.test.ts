import { afterEach, describe, expect, it, vi } from "vitest";
import { randomUuid } from "./random-uuid";

describe("randomUuid", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("uses the native secure-context implementation when available", () => {
    vi.stubGlobal("crypto", { randomUUID: () => "11111111-1111-4111-8111-111111111111" });
    expect(randomUuid()).toBe("11111111-1111-4111-8111-111111111111");
  });

  it("builds a valid v4 UUID from getRandomValues on LAN HTTP origins", () => {
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array) => {
        bytes.forEach((_, index) => { bytes[index] = index; });
        return bytes;
      },
    });
    expect(randomUuid()).toBe("00010203-0405-4607-8809-0a0b0c0d0e0f");
  });
});
