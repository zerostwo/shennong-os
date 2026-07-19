import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ChatView } from "./chat-view";

const mocks = vi.hoisted(() => ({ runtimeProvider: vi.fn(), thread: vi.fn(), setProviderId: vi.fn(), setThinkingLevel: vi.fn(), getSession: vi.fn() }));

vi.mock("@/components/app-shell", () => ({
  AppShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock("@/components/assistant-ui/runtime-provider", () => ({
  ShennongRuntimeProvider: (props: { projectId?: string; children: React.ReactNode }) => {
    mocks.runtimeProvider(props);
    return <div data-testid="runtime-provider">{props.children}</div>;
  },
  useShennongAssistantRuntime: () => ({
    providers: [{ id: "provider-1", name: "Local", model: "qwen", enabled: true }],
    providerId: "provider-1",
    setProviderId: mocks.setProviderId,
    thinkingLevel: "medium",
    setThinkingLevel: mocks.setThinkingLevel,
  }),
}));
vi.mock("@/components/assistant-ui/thread", () => ({ ShennongThread: (props: unknown) => { mocks.thread(props); return <div>Assistant thread</div>; }, ThreadSkillSelector: () => <button>Skills</button> }));
vi.mock("@/lib/api/adapter", () => ({ getSession: mocks.getSession }));

describe("ChatView conversation scope", () => {
  beforeEach(() => { mocks.runtimeProvider.mockClear(); mocks.thread.mockClear(); mocks.getSession.mockReset(); mocks.getSession.mockResolvedValue({ authenticated: true }); });

  it("mounts a personal Agent runtime without requiring a Project", async () => {
    render(<ChatView />);

    expect(await screen.findByTestId("runtime-provider")).toBeInTheDocument();
    expect(screen.getByText("Assistant thread")).toBeInTheDocument();
    expect(mocks.runtimeProvider).toHaveBeenCalledWith(expect.objectContaining({ projectId: undefined }));
  });

  it("mounts the Agent runtime with the explicit Project", async () => {
    render(<ChatView projectId="project-1" initialPrompt="Inspect project://current/resources/resource-1" />);

    expect(await screen.findByTestId("runtime-provider")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Skills" })).toBeInTheDocument();
    expect(mocks.runtimeProvider).toHaveBeenCalledWith(expect.objectContaining({ projectId: "project-1" }));
    expect(mocks.thread).toHaveBeenCalledWith(expect.objectContaining({ projectId: "project-1", initialPrompt: "Inspect project://current/resources/resource-1" }));
  });

  it("keeps the protected Agent runtime unmounted for guests", async () => {
    mocks.getSession.mockResolvedValue({ authenticated: false });
    render(<ChatView />);
    expect(await screen.findByRole("heading", { name: "Sign in to chat with Shennong" })).toBeInTheDocument();
    expect(screen.queryByTestId("runtime-provider")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Sign in to start a chat" })).toBeInTheDocument();
  });
});
