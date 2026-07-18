import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SettingsDialog } from "./settings-dialog";

const mocks = vi.hoisted(() => ({
  createAiProvider: vi.fn(),
  listAiProviders: vi.fn(),
}));

vi.mock("@/lib/api/adapter", async () => ({
  ...(await vi.importActual<typeof import("@/lib/api/adapter")>("@/lib/api/adapter")),
  createAiProvider: mocks.createAiProvider,
  listAiProviders: mocks.listAiProviders,
}));

describe("SettingsDialog V1 model contract", () => {
  beforeEach(() => {
    mocks.createAiProvider.mockReset();
    mocks.listAiProviders.mockReset();
    mocks.listAiProviders.mockResolvedValue([]);
    mocks.createAiProvider.mockResolvedValue({ id: "provider-1" });
  });

  it("accepts an explicit model ID without exposing unsupported discovery controls", async () => {
    const user = userEvent.setup();
    render(
      <SettingsDialog
        open
        onOpenChange={vi.fn()}
        initialSection="models"
        session={{ authenticated: true, user_id: "user-1", role: "user" }}
      />,
    );

    await user.click(await screen.findByRole("button", { name: "Add model" }));
    expect(screen.queryByRole("button", { name: /load models/i })).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("API key"), "test-key");
    await user.type(screen.getByLabelText("Model ID"), "gpt-v1-test");
    await user.click(screen.getByRole("button", { name: "Save model" }));

    await waitFor(() => expect(mocks.createAiProvider).toHaveBeenCalledWith(expect.objectContaining({
      provider_kind: "openai",
      base_url: "https://api.openai.com/v1",
      model: "gpt-v1-test",
      api_key: "test-key",
    })));
  });

  it("keeps Skills outside Settings and exposes the requested preference sections", () => {
    render(<SettingsDialog open onOpenChange={vi.fn()} session={{ authenticated: true, user_id: "user-1", role: "user" }} />);
    expect(screen.getByRole("button", { name: "Account" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Personalization" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Keyboard" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Skills" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Settings" })).toHaveClass("sr-only");
  });
});
