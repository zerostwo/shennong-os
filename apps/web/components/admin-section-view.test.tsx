import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AdminSectionView } from "./admin-section-view";

const api = vi.hoisted(() => ({
  getAdminOverview: vi.fn(), getHealth: vi.fn(), getRegistrationPolicy: vi.fn(), getSystemDependencies: vi.fn(),
  listAdminModelProviders: vi.fn(), listAuditEvents: vi.fn(), listProviders: vi.fn(), listUsers: vi.fn(), updateRegistrationPolicy: vi.fn(),
}));

vi.mock("@/lib/api/adapter", () => api);
vi.mock("./app-shell", () => ({
  AppShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  TopBar: ({ title }: { title: string }) => <h1>{title}</h1>,
  TinyBadge: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

describe("AdminSectionView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getAdminOverview.mockResolvedValue({ users: 8, active_users: 7, projects: 3, threads: 11, runs: 5, active_runs: 1, model_providers: 2, enabled_model_providers: 1 });
    api.getHealth.mockResolvedValue({ backends: { postgres: "ok" } });
    api.listAuditEvents.mockResolvedValue([{ id: 1, created_at: "2026-07-19T00:00:00Z", action: "invite.create", target_type: "invite", target_id: "i-1", details: { uses: 1 } }]);
    api.listUsers.mockResolvedValue([]);
  });

  it("renders real dashboard counts and structured health data", async () => {
    const { container } = render(<AdminSectionView section="dashboard" />);
    expect(await screen.findByText("8")).toBeInTheDocument();
    expect(screen.getByText("Postgres")).toBeInTheDocument();
    expect(screen.getByText("ok")).toBeInTheDocument();
    expect(container.textContent).not.toContain('{"postgres"');
  });

  it("links users to the activated detail route", async () => {
    api.listUsers.mockResolvedValue([{ id: "user-1", display_name: "Research Admin", email: "admin@example.org", role: "admin", status: "active", owned_projects: 2, active_sessions: 1, updated_at: "2026-07-19T00:00:00Z" }]);
    render(<AdminSectionView section="users" />);
    expect(await screen.findByRole("link", { name: "Research Admin" })).toHaveAttribute("href", "/admin/users/user-1");
  });
});
