import type { NextRequest } from "next/server";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const AGENT_URL = process.env.SHENNONG_AGENT_INTERNAL_URL ?? "http://127.0.0.1:8080/api/v1/agent";
const MAX_AGENT_REQUEST_BYTES = 2 * 1024 * 1024;

export async function POST(request: NextRequest) {
  const contentLength = Number(request.headers.get("content-length") ?? 0);
  if (!Number.isFinite(contentLength) || contentLength < 0 || contentLength > MAX_AGENT_REQUEST_BYTES) {
    return Response.json({ code: "request_too_large", message: "Agent request is too large" }, { status: 413 });
  }
  const body = await request.arrayBuffer();
  if (body.byteLength > MAX_AGENT_REQUEST_BYTES) return Response.json({ code: "request_too_large", message: "Agent request is too large" }, { status: 413 });

  const headers = new Headers({
    accept: "text/event-stream",
    "content-type": "application/json",
    "x-shennong-ui": "assistant-ui",
  });
  for (const name of ["cookie", "origin", "x-csrf-token", "x-shennong-project-id", "x-shennong-provider-id", "x-shennong-thinking-level"]) {
    const value = request.headers.get(name);
    if (value) headers.set(name, value);
  }
  if (!headers.has("x-csrf-token")) {
    const csrf = request.cookies.get("shennong_os_csrf")?.value;
    if (csrf) headers.set("x-csrf-token", csrf);
  }
  try {
    const response = await fetch(AGENT_URL, {
      method: "POST",
      headers,
      body,
      redirect: "manual",
      signal: request.signal,
    });
    const responseHeaders = new Headers({
      "content-type": response.headers.get("content-type") ?? "text/event-stream; charset=utf-8",
      "cache-control": "no-cache, no-transform",
      connection: "keep-alive",
      "x-accel-buffering": "no",
      "x-content-type-options": "nosniff",
    });
    return new Response(response.body, { status: response.status, headers: responseHeaders });
  } catch (error) {
    console.error("Shennong Agent Gateway request failed", error);
    return Response.json({ code: "agent_unavailable", message: "Shennong Agent is temporarily unavailable" }, { status: 503 });
  }
}
