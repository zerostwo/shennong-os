import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ChatView } from "./chat-view";

const mocks = vi.hoisted(() => ({ runtimeProvider: vi.fn(), thread: vi.fn() }));

vi.mock("@/components/app-shell", () => ({
  AppShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock("@/components/assistant-ui/runtime-provider", () => ({
  ShennongRuntimeProvider: (props: { projectId?: string; children: React.ReactNode }) => {
    mocks.runtimeProvider(props);
    return <div data-testid="runtime-provider">{props.children}</div>;
  },
}));
vi.mock("@/components/assistant-ui/thread", () => ({ ShennongThread: (props: unknown) => { mocks.thread(props); return <div>Assistant thread</div>; }, ThreadSkillSelector: () => <button>Skills</button> }));

describe("ChatView conversation scope", () => {
  beforeEach(() => { mocks.runtimeProvider.mockClear(); mocks.thread.mockClear(); });

  it("mounts a personal Agent runtime without requiring a Project", () => {
    render(<ChatView />);

    expect(screen.getByTestId("runtime-provider")).toBeInTheDocument();
    expect(screen.getByText("Assistant thread")).toBeInTheDocument();
    expect(mocks.runtimeProvider).toHaveBeenCalledWith(expect.objectContaining({ projectId: undefined }));
  });

  it("mounts the Agent runtime with the explicit Project", () => {
    render(<ChatView projectId="project-1" initialPrompt="Inspect project://current/resources/resource-1" />);

    expect(screen.getByTestId("runtime-provider")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Skills" })).toBeInTheDocument();
    expect(mocks.runtimeProvider).toHaveBeenCalledWith(expect.objectContaining({ projectId: "project-1" }));
    expect(mocks.thread).toHaveBeenCalledWith(expect.objectContaining({ projectId: "project-1", initialPrompt: "Inspect project://current/resources/resource-1" }));
  });
});
