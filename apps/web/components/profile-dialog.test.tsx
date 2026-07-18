import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProfileDialog } from "./profile-dialog";

const updateProfile = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api/adapter", () => ({ updateProfile }));

describe("ProfileDialog", () => {
  it("persists display name and username through the profile API", async () => {
    const user = userEvent.setup();
    const onSaved = vi.fn();
    updateProfile.mockResolvedValue({ authenticated: true, user_id: "user-1", role: "user", scopes: [], display_name: "Songqi Duan", username: "duansq" });
    render(<ProfileDialog open onOpenChange={vi.fn()} onSaved={onSaved} session={{ authenticated: true, user_id: "user-1", role: "user", scopes: [], display_name: "Researcher", username: "researcher" }} />);
    await user.clear(screen.getByLabelText("Display name"));
    await user.type(screen.getByLabelText("Display name"), "Songqi Duan");
    await user.clear(screen.getByLabelText("Username"));
    await user.type(screen.getByLabelText("Username"), "DuanSQ");
    await user.click(screen.getByRole("button", { name: "Save profile" }));
    await waitFor(() => expect(updateProfile).toHaveBeenCalledWith({ display_name: "Songqi Duan", username: "duansq", avatar_url: "" }));
    expect(onSaved).toHaveBeenCalledWith(expect.objectContaining({ username: "duansq" }));
  });
});
