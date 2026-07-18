import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ThreadSkillSelector } from "./thread";

const mocks = vi.hoisted(() => ({
  runtime: { activeThreadId: "thread-1", hasPersistedThread: true, projectId: "project-1", refreshThreads: vi.fn() },
  list: vi.fn(),
  enable: vi.fn(),
  disable: vi.fn(),
}));

vi.mock("@/components/assistant-ui/runtime-provider", () => ({
  useShennongAssistantRuntime: () => mocks.runtime,
}));

vi.mock("@/lib/api/adapter", () => ({
  listThreadSkills: mocks.list,
  enableThreadSkill: mocks.enable,
  disableThreadSkill: mocks.disable,
}));

const builtInSkill = {
  id: "skill-1",
  slug: "inspect-biomedical-input",
  name: "Inspect biomedical input",
  description: "Inspect governed inputs",
  sourceKind: "built_in" as const,
  trustLevel: "builtin_signed" as const,
  status: "active" as const,
  revision: 1,
  content: "Inspect first.",
  isBuiltin: true,
  enabled: false,
  selectedVersion: null,
  createdAt: "",
  updatedAt: "",
  raw: {},
};

describe("ThreadSkillSelector", () => {
  beforeEach(() => {
    mocks.runtime.hasPersistedThread = true;
    mocks.list.mockReset().mockResolvedValue([builtInSkill]);
    mocks.enable.mockReset().mockResolvedValue(undefined);
    mocks.disable.mockReset().mockResolvedValue(undefined);
  });

  it("lists active server Skills and pins one through the thread contract", async () => {
    render(<ThreadSkillSelector />);
    const checkbox = await screen.findByRole("checkbox", { name: /Inspect biomedical input/ });
    fireEvent.click(checkbox);
    await waitFor(() => expect(mocks.enable).toHaveBeenCalledWith("thread-1", "skill-1"));
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it("does not offer a fake selection before the thread exists", () => {
    mocks.runtime.hasPersistedThread = false;
    render(<ThreadSkillSelector />);
    expect(screen.getByRole("button", { name: "Skills" })).toBeDisabled();
    expect(mocks.list).not.toHaveBeenCalled();
  });
});
