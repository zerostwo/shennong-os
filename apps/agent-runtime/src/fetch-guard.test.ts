import assert from "node:assert/strict";
import test from "node:test";
import { createProviderFetchGuard, isPublicAddress } from "./fetch-guard.js";

test("public-address classification rejects local and reserved networks", () => {
  assert.equal(isPublicAddress("127.0.0.1"), false);
  assert.equal(isPublicAddress("169.254.169.254"), false);
  assert.equal(isPublicAddress("10.0.0.1"), false);
  assert.equal(isPublicAddress("::1"), false);
  assert.equal(isPublicAddress("93.184.216.34"), true);
});

test("remote provider guard blocks DNS rebinding and targets outside the configured base", async () => {
  let transports = 0;
  const guarded = createProviderFetchGuard({
    getPolicy: () => ({ kind: "openai", baseUrl: "https://provider.example/v1" }),
    resolveHost: async () => [
      { address: "93.184.216.34", family: 4 },
      { address: "127.0.0.1", family: 4 },
    ],
    transport: async () => {
      transports += 1;
      return new Response("ok");
    },
  });

  await assert.rejects(guarded("https://provider.example/v1/chat/completions"), /provider_host_not_public/);
  await assert.rejects(guarded("https://provider.example.evil/v1/chat/completions"), /provider_target_outside_base/);
  assert.equal(transports, 0);
});

test("Ollama is limited to the local OpenAI-compatible endpoint", async () => {
  let target = "";
  const guarded = createProviderFetchGuard({
    getPolicy: () => ({ kind: "ollama", baseUrl: "http://localhost:11434/v1" }),
    resolveHost: async () => [{ address: "127.0.0.1", family: 4 }],
    transport: async (request) => {
      target = request.url;
      return new Response("{}", { status: 200 });
    },
  });
  const response = await guarded("http://localhost:11434/v1/chat/completions", { method: "POST" });
  assert.equal(response.status, 200);
  assert.equal(target, "http://localhost:11434/v1/chat/completions");

  const unsafe = createProviderFetchGuard({
    getPolicy: () => ({ kind: "ollama", baseUrl: "http://localhost:11435/v1" }),
  });
  await assert.rejects(unsafe("http://localhost:11435/v1/chat/completions"), /ollama_url_not_allowed/);
});

test("provider redirects are never followed", async () => {
  const guarded = createProviderFetchGuard({
    getPolicy: () => ({ kind: "deepseek", baseUrl: "https://provider.example/v1" }),
    resolveHost: async () => [{ address: "93.184.216.34", family: 4 }],
    transport: async () => new Response(null, { status: 302, headers: { location: "http://127.0.0.1/" } }),
  });
  await assert.rejects(guarded("https://provider.example/v1/chat/completions"), /provider_redirect_blocked/);
});
