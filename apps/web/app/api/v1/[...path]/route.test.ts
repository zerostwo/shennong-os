import { NextRequest } from "next/server";
import { describe, expect, it } from "vitest";

import { browserForwardedHeaders } from "@/lib/browser-bff-headers";

function request(path: string, method = "POST") {
  return new NextRequest(`http://shennong.test/api/v1/${path}`, {
    method,
    headers: {
      cookie: "shennong_os_csrf=csrf-cookie",
      "last-event-id": "42",
      "x-shennong-bootstrap-token": "bootstrap-secret",
    },
  });
}

describe("browser BFF request headers", () => {
  it("forwards the bootstrap token only to the setup endpoint", () => {
    const setupHeaders = browserForwardedHeaders(request("setup/admin"), "setup/admin");
    expect(setupHeaders.get("x-shennong-bootstrap-token")).toBe("bootstrap-secret");

    const registrationHeaders = browserForwardedHeaders(request("auth/register"), "auth/register");
    expect(registrationHeaders.has("x-shennong-bootstrap-token")).toBe(false);
  });

  it("does not forward the bootstrap token for a non-POST setup request", () => {
    const headers = browserForwardedHeaders(request("setup/admin", "GET"), "setup/admin");
    expect(headers.has("x-shennong-bootstrap-token")).toBe(false);
  });

  it("forwards Last-Event-ID for authenticated SSE replay", () => {
    const headers = browserForwardedHeaders(
      request("runs/run-1/events/stream", "GET"),
      "runs/run-1/events/stream",
    );
    expect(headers.get("last-event-id")).toBe("42");
  });

  it("forwards upload metadata only to the exact Project upload route", () => {
    const uploadRequest = new NextRequest(
      "http://shennong.test/api/v1/projects/project-1/uploads",
      {
        method: "POST",
        headers: {
          "content-length": "4",
          "content-type": "text/tab-separated-values",
          "x-filename": "matrix.tsv",
        },
        body: "test",
      },
    );
    const uploadHeaders = browserForwardedHeaders(
      uploadRequest,
      "projects/project-1/uploads",
    );
    expect(uploadHeaders.get("x-filename")).toBe("matrix.tsv");
    expect(uploadHeaders.get("content-length")).toBe("4");

    const registrationHeaders = browserForwardedHeaders(
      uploadRequest,
      "projects/project-1/uploads/register",
    );
    expect(registrationHeaders.has("x-filename")).toBe(false);
    expect(registrationHeaders.has("content-length")).toBe(false);
    expect(registrationHeaders.has("x-shennong-os-actor-id")).toBe(false);
    expect(registrationHeaders.has("x-shennong-os-project-id")).toBe(false);
  });
});
