import assert from "node:assert/strict";
import test from "node:test";
import { HttpOsInternalClient } from "./os-client.js";

test("OS callback errors preserve structured code and message", async () => {
  const client = new HttpOsInternalClient(
    "http://os-server:8080",
    "s".repeat(32),
    async () => new Response(JSON.stringify({
      error: { code: "data_plane_rejected", message: "Shennong DB rejected the governed request" },
    }), { status: 502, headers: { "content-type": "application/json" } }),
  );

  await assert.rejects(
    () => client.finishRun("run-1", undefined, { code: "test", message: "test" }),
    /data_plane_rejected: Shennong DB rejected the governed request/,
  );
});
