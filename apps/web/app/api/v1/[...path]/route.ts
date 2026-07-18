import type { NextRequest } from "next/server";

import { browserForwardedHeaders } from "@/lib/browser-bff-headers";
import { isBrowserRouteAllowed } from "@/lib/browser-bff-route-policy";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

const MAX_REQUEST_BYTES = 2 * 1024 * 1024;
const MAX_UPLOAD_BYTES = 50 * 1024 * 1024 * 1024;
const INTERNAL_API = process.env.SHENNONG_API_INTERNAL_URL ?? "http://127.0.0.1:8001";

type Context = { params: Promise<{ path: string[] }> };

function responseHeaders(source: Headers) {
  const headers = new Headers();
  for (const name of ["accept-ranges", "cache-control", "content-disposition", "content-range", "content-type", "etag", "location", "vary", "x-accel-buffering"]) {
    const value = source.get(name);
    if (value) headers.set(name, value);
  }
  const cookieValues = (source as Headers & { getSetCookie?: () => string[] }).getSetCookie?.()
    ?? (source.get("set-cookie") ? [source.get("set-cookie") as string] : []);
  for (const value of cookieValues) headers.append("set-cookie", value);
  headers.set("x-content-type-options", "nosniff");
  return headers;
}

async function proxy(request: NextRequest, context: Context): Promise<Response> {
  const { path } = await context.params;
  if (!path.length || path.length > 8 || path.some((segment) => !segment || segment === "." || segment === ".." || segment.includes("/") || segment.includes("\\") || segment.length > 180)) {
    return Response.json({ code: "invalid_route", message: "Invalid API route" }, { status: 400 });
  }
  if (request.nextUrl.search.length > 4096) return Response.json({ code: "query_too_large", message: "Query string is too large" }, { status: 414 });

  const method = request.method.toUpperCase();
  const normalizedPath = path.join("/");
  if (!isBrowserRouteAllowed(normalizedPath, method)) return Response.json({ code: "route_not_allowed", message: "This API route is not exposed by Shennong OS" }, { status: 404 });

  const isUpload = method === "POST" && /^projects\/[^/]+\/uploads$/.test(normalizedPath);
  const rawContentLength = request.headers.get("content-length");
  const contentLength = Number(rawContentLength ?? 0);
  const requestLimit = isUpload ? MAX_UPLOAD_BYTES : MAX_REQUEST_BYTES;
  if (isUpload && (!rawContentLength || !Number.isFinite(contentLength) || contentLength <= 0)) {
    return Response.json({ code: "upload_length_required", message: "A valid Content-Length header is required for uploads" }, { status: 411 });
  }
  if (!Number.isFinite(contentLength) || contentLength < 0 || contentLength > requestLimit) {
    return Response.json({ code: "request_too_large", message: "Request body is too large" }, { status: 413 });
  }
  const body = method === "GET" || method === "HEAD"
    ? undefined
    : isUpload
      ? request.body ?? undefined
      : await request.arrayBuffer();
  if (body instanceof ArrayBuffer && body.byteLength > requestLimit) {
    return Response.json({ code: "request_too_large", message: "Request body is too large" }, { status: 413 });
  }

  const encodedPath = path.map((segment) => encodeURIComponent(segment)).join("/");
  const target = new URL(`/api/v1/${encodedPath}${request.nextUrl.search}`, INTERNAL_API);
  try {
    const init: RequestInit & { duplex?: "half" } = {
      method,
      headers: browserForwardedHeaders(request, normalizedPath),
      body,
      redirect: "manual",
      signal: request.signal,
      ...(isUpload ? { duplex: "half" as const } : {}),
    };
    const response = await fetch(target, init);
    return new Response(response.body, {
      status: response.status,
      statusText: response.statusText,
      headers: responseHeaders(response.headers),
    });
  } catch (error) {
    console.error("Shennong OS API request failed", error);
    return Response.json({ code: "api_unavailable", message: "Shennong OS API is temporarily unavailable" }, { status: 503 });
  }
}

export const GET = proxy;
export const HEAD = proxy;
export const POST = proxy;
export const PUT = proxy;
export const PATCH = proxy;
export const DELETE = proxy;
