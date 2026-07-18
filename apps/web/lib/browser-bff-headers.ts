import type { NextRequest } from "next/server";

export function browserForwardedHeaders(request: NextRequest, normalizedPath: string) {
  const headers = new Headers();
  for (const name of ["accept", "content-type", "cookie", "idempotency-key", "if-match", "last-event-id", "origin", "range", "x-csrf-token"]) {
    const value = request.headers.get(name);
    if (value) headers.set(name, value);
  }
  if (!headers.has("x-csrf-token")) {
    const csrf = request.cookies.get("shennong_os_csrf")?.value;
    if (csrf) headers.set("x-csrf-token", csrf);
  }
  if (normalizedPath === "setup/admin" && request.method.toUpperCase() === "POST") {
    const bootstrapToken = request.headers.get("x-shennong-bootstrap-token");
    if (bootstrapToken) headers.set("x-shennong-bootstrap-token", bootstrapToken);
  }
  if (
    /^projects\/[^/]+\/uploads$/.test(normalizedPath)
    && request.method.toUpperCase() === "POST"
  ) {
    for (const name of ["content-length", "x-filename"]) {
      const value = request.headers.get(name);
      if (value) headers.set(name, value);
    }
  }
  headers.set("x-forwarded-proto", request.nextUrl.protocol.replace(":", ""));
  headers.set("x-forwarded-host", request.headers.get("host") ?? request.nextUrl.host);
  return headers;
}
