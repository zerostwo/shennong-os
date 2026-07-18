import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AuthView } from "./auth-view";

const mocks = vi.hoisted(() => ({
  getPublicConfig: vi.fn(),
  getSetupStatus: vi.fn(),
  registerUser: vi.fn(),
  setupAdmin: vi.fn(),
  signIn: vi.fn(),
}));

vi.mock("@/lib/api/adapter", () => ({
  ...mocks,
  ShennongApiError: class ShennongApiError extends Error {},
}));

describe("AuthView V1 contract", () => {
  beforeEach(() => {
    Object.values(mocks).forEach((mock) => mock.mockReset());
    mocks.getSetupStatus.mockResolvedValue({ needs_setup: false });
    mocks.getPublicConfig.mockResolvedValue({ registration_mode: "disabled", invite_required: false });
    mocks.setupAdmin.mockResolvedValue({ authenticated: true, user_id: "admin-1", role: "admin", scopes: [] });
    mocks.registerUser.mockResolvedValue({ authenticated: true, user_id: "user-1", role: "user", scopes: [] });
    mocks.signIn.mockResolvedValue({ authenticated: true, user_id: "user-1", role: "user", scopes: [] });
    window.history.replaceState({}, "", "/auth/sign-in");
  });

  it("shows and requires an invitation code only for invite-only registration", async () => {
    mocks.getPublicConfig.mockResolvedValue({ registration_mode: "invite_only", invite_required: true });
    window.history.replaceState({}, "", "/auth/sign-in?mode=register");

    render(<AuthView />);

    expect(await screen.findByRole("heading", { name: "Create your account" })).toBeInTheDocument();
    expect(screen.getByLabelText("Invitation code")).toBeRequired();
    expect(screen.getByText("Early access registration requires an invitation code.")).toBeInTheDocument();
  });

  it("does not show an invitation field when public registration is open", async () => {
    mocks.getPublicConfig.mockResolvedValue({ registration_mode: "open", invite_required: false });
    window.history.replaceState({}, "", "/auth/sign-in?mode=register");

    render(<AuthView />);

    expect(await screen.findByRole("heading", { name: "Create your account" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Invitation code")).not.toBeInTheDocument();
    expect(screen.getByText("Create an account to start a governed biomedical analysis workspace.")).toBeInTheDocument();
  });

  it("uses the authenticated bootstrap response without a second sign-in", async () => {
    const user = userEvent.setup();
    mocks.getSetupStatus.mockResolvedValue({ needs_setup: true });

    render(<AuthView />);

    expect(await screen.findByRole("heading", { name: "Create the administrator" })).toBeInTheDocument();
    await user.type(screen.getByLabelText("Display name"), "V1 Admin");
    await user.type(screen.getByLabelText("Email"), "admin@example.test");
    await user.type(screen.getByLabelText("Password"), "correct-horse-battery-staple");
    await user.type(screen.getByLabelText("Bootstrap token"), "bootstrap-secret");
    await user.click(screen.getByRole("button", { name: "Create administrator" }));

    await waitFor(() => expect(mocks.setupAdmin).toHaveBeenCalledWith(
      "V1 Admin",
      "admin@example.test",
      "correct-horse-battery-staple",
      "bootstrap-secret",
    ));
    expect(mocks.signIn).not.toHaveBeenCalled();
    expect(await screen.findByRole("heading", { name: "Administrator ready" })).toBeInTheDocument();
  });
});
