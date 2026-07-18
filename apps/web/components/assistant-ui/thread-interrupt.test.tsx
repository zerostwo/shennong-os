import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { InterruptPanel } from "./thread";

const mocks = vi.hoisted(() => ({
  submit: vi.fn(async () => undefined),
  interrupts: [{
    id: "00000000-0000-4000-8000-000000000043",
    reason: "tool_call",
    message: "Approve project.write_file (project.write) for this project?",
    toolCallId: "tool-write-1",
  }],
}));

vi.mock("@assistant-ui/react-ag-ui", async (importOriginal) => {
  const original = await importOriginal<typeof import("@assistant-ui/react-ag-ui")>();
  return {
    ...original,
    useAgUiInterrupts: () => mocks.interrupts,
    useAgUiSubmitInterruptResponses: () => mocks.submit,
  };
});

describe("native AG-UI interrupt approval", () => {
  beforeEach(() => mocks.submit.mockClear());

  it("submits the exact resolved response through assistant-ui's native hook", async () => {
    render(<InterruptPanel />);
    expect(screen.getByText("Approval required")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledWith([{
      interruptId: "00000000-0000-4000-8000-000000000043",
      status: "resolved",
      payload: { approved: true },
    }]));
  });

  it("rejects without sending tool arguments or a payload", async () => {
    render(<InterruptPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Reject" }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledWith([{
      interruptId: "00000000-0000-4000-8000-000000000043",
      status: "cancelled",
    }]));
  });
});
