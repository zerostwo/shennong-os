import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ChatView } from "./chat-view";

const mocks = vi.hoisted(() => ({ runtimeProvider: vi.fn() }));

vi.mock("@/components/app-shell", () => ({
  AppShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock("@/components/assistant-ui/runtime-provider", () => ({
  ShennongRuntimeProvider: (props: { projectId?: string; children: React.ReactNode }) => {
    mocks.runtimeProvider(props);
    return <div data-testid="runtime-provider">{props.children}</div>;
  },
}));
vi.mock("@/components/assistant-ui/thread", () => ({ ShennongThread: () => <div>Assistant thread</div>, ThreadSkillSelector: () => <button>Skills</button> }));

describe("ChatView project boundary", () => {
  beforeEach(() => mocks.runtimeProvider.mockClear());

  it("shows a Project CTA without mounting an Agent runtime", () => {
    render(<ChatView />);

    expect(screen.getByRole("heading", { name: "Choose a research project" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open projects" })).toHaveAttribute("href", "/projects");
    expect(screen.queryByTestId("runtime-provider")).not.toBeInTheDocument();
    expect(mocks.runtimeProvider).not.toHaveBeenCalled();
  });

  it("mounts the Agent runtime with the explicit Project", () => {
    render(<ChatView projectId="project-1" />);

    expect(screen.getByTestId("runtime-provider")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Skills" })).toBeInTheDocument();
    expect(mocks.runtimeProvider).toHaveBeenCalledWith(expect.objectContaining({ projectId: "project-1" }));
  });
});
