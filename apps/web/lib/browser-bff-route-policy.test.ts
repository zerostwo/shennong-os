import { describe, expect, it } from "vitest";

import { isBrowserRouteAllowed } from "./browser-bff-route-policy";

describe("Shennong OS browser BFF route policy", () => {
  it.each([
    ["GET", "auth/registration-policy"],
    ["POST", "setup/admin"],
    ["GET", "projects/project-1/members"],
    ["PUT", "projects/project-1/members/user-1"],
    ["POST", "projects/project-1/jobs"],
    ["POST", "projects/project-1/sessions"],
    ["GET", "projects/project-1/graph/subgraph"],
    ["POST", "projects/project-1/uploads"],
    ["POST", "projects/project-1/uploads/register"],
    ["POST", "sessions/session-1/launch"],
    ["POST", "threads/thread-1/runs"],
    ["GET", "threads/thread-1/runs/active"],
    ["GET", "runs/run-1/events"],
    ["GET", "runs/run-1/events/stream"],
    ["PUT", "runs/run-1/plan"],
    ["PUT", "threads/thread-1/skills/skill-1"],
    ["GET", "threads/thread-1/skills"],
    ["POST", "skills/skill-1/versions"],
    ["GET", "resources/resource-1/graph-context"],
    ["POST", "query"],
  ])("allows browser route %s /api/v1/%s", (method, path) => {
    expect(isBrowserRouteAllowed(path, method)).toBe(true);
  });

  it.each([
    ["POST", "agent"],
    ["POST", "agent/runs"],
    ["POST", "agent/runs/run-1/events"],
    ["POST", "agent/runs/run-1/finish"],
    ["POST", "agent/runs/run-1/approvals/verify"],
    ["POST", "agent/runs/run-1/tools"],
    ["GET", "__shennong/launch"],
    ["GET", "v1/sessions/session-1/proxy"],
    ["POST", "uploads"],
    ["POST", "projects/project-1/uploads/upload-1"],
    ["GET", "users"],
    ["GET", "auth/tokens"],
    ["GET", "projects/project-1/unknown"],
    ["POST", "projects/project-1/graph/subgraph"],
    ["PATCH", "projects/project-1/resources"],
    ["DELETE", "skills/skill-1"],
    ["POST", "skills/generate"],
    ["DELETE", "providers/provider-1/models"],
  ])("rejects non-browser or unsupported route %s /api/v1/%s", (method, path) => {
    expect(isBrowserRouteAllowed(path, method)).toBe(false);
  });
});
