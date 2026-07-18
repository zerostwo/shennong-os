import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ComputeView } from "./compute-view";

const mocks = vi.hoisted(() => ({
  launchRuntimeSession: vi.fn(),
  listRuntimeJobs: vi.fn(),
  listRuntimeSessions: vi.fn(),
  startRuntimeSession: vi.fn(),
  stopRuntimeSession: vi.fn(),
  cancelRuntimeJob: vi.fn(),
}));

vi.mock("next/navigation", () => ({
  usePathname: () => "/projects/project-1/compute",
  useRouter: () => ({ push: vi.fn() }),
}));
vi.mock("@/lib/api/adapter", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api/adapter")>("@/lib/api/adapter");
  return {
    ...actual,
    getSession: async () => ({ authenticated: true, user_id: "user-1", role: "user", scopes: [] }),
    getPublicConfig: async () => ({ registration_mode: "invite_only" }),
    launchRuntimeSession: mocks.launchRuntimeSession,
    listRuntimeJobs: mocks.listRuntimeJobs,
    listRuntimeSessions: mocks.listRuntimeSessions,
    startRuntimeSession: mocks.startRuntimeSession,
    stopRuntimeSession: mocks.stopRuntimeSession,
    cancelRuntimeJob: mocks.cancelRuntimeJob,
  };
});

describe("ComputeView IDE launch", () => {
  beforeEach(() => {
    Object.values(mocks).forEach((mock) => mock.mockReset());
    mocks.listRuntimeJobs.mockResolvedValue([]);
    mocks.listRuntimeSessions.mockResolvedValue([{
      id: "session-1",
      projectId: "project-1",
      kind: "rstudio",
      status: "running",
      createdAt: "2026-07-18T12:00:00Z",
      expiresAt: "2026-07-18T20:00:00Z",
    }]);
  });

  it("requests a one-time ticket on Open and never renders the ticket", async () => {
    const user = userEvent.setup();
    const replace = vi.fn();
    const close = vi.fn();
    const ideWindow = { opener: window, location: { replace }, close } as unknown as Window;
    const open = vi.spyOn(window, "open").mockReturnValue(ideWindow);
    const launchUrl = "https://ide.example.test/__shennong/launch?ticket=one-time-secret";
    mocks.launchRuntimeSession.mockResolvedValue({ launchUrl, expiresAt: "2026-07-18T12:01:00Z" });

    render(<ComputeView projectId="project-1" />);
    await user.click(await screen.findByRole("button", { name: "Open RStudio Server" }));

    expect(open).toHaveBeenCalledWith("about:blank", "_blank");
    expect(ideWindow.opener).toBeNull();
    expect(mocks.launchRuntimeSession).toHaveBeenCalledWith("session-1");
    await waitFor(() => expect(replace).toHaveBeenCalledWith(launchUrl));
    expect(screen.queryByText(/one-time-secret/)).not.toBeInTheDocument();
    expect(close).not.toHaveBeenCalled();
  });

  it("does not mint a ticket when the browser blocks the new window", async () => {
    const user = userEvent.setup();
    vi.spyOn(window, "open").mockReturnValue(null);

    render(<ComputeView projectId="project-1" />);
    await user.click(await screen.findByRole("button", { name: "Open RStudio Server" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("blocked the IDE window");
    expect(mocks.launchRuntimeSession).not.toHaveBeenCalled();
  });

  it("closes a failed launch window without reflecting sensitive response text", async () => {
    const user = userEvent.setup();
    const close = vi.fn();
    const ideWindow = { opener: window, location: { replace: vi.fn() }, close } as unknown as Window;
    vi.spyOn(window, "open").mockReturnValue(ideWindow);
    mocks.launchRuntimeSession.mockRejectedValue(new Error("ticket=must-not-render"));

    render(<ComputeView projectId="project-1" />);
    await user.click(await screen.findByRole("button", { name: "Open RStudio Server" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("IDE could not be opened");
    expect(screen.queryByText(/must-not-render/)).not.toBeInTheDocument();
    expect(close).toHaveBeenCalledOnce();
  });
});
