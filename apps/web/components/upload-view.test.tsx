import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { UploadView } from "./upload-view";

const mocks = vi.hoisted(() => ({
  registerProjectFiles: vi.fn(async () => ({ resource: {}, resourceId: "resource-1", resourceName: "Resource 1", uri: "project://current/resources/resource-1", filenames: ["matrix.tsv"] })),
  buildProjectUploadPrompt: vi.fn(() => "Inspect project://current/resources/resource-1"),
  push: vi.fn(),
}));

vi.mock("next/navigation", () => ({ usePathname: () => "/projects/project-1/uploads/new", useRouter: () => ({ push: mocks.push }) }));
vi.mock("@/lib/project-upload", () => ({ registerProjectFiles: mocks.registerProjectFiles, buildProjectUploadPrompt: mocks.buildProjectUploadPrompt }));
vi.mock("@/lib/api/adapter", () => ({ getSession: async () => ({ authenticated: false, user_id: "", role: "", scopes: [] }), getPublicConfig: async () => ({ registration_mode: "disabled" }), getHealth: async () => ({ status: "ok" }), listIngestionJobs: async () => [], signOut: async () => undefined }));

describe("UploadView", () => {
  it("registers files with minimal context and hands a durable reference to Project chat", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const invalidate = vi.spyOn(client, "invalidateQueries");
    render(<QueryClientProvider client={client}><UploadView projectId="project-1" /></QueryClientProvider>);
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(["real"], "matrix.tsv", { type: "text/tab-separated-values" });
    fireEvent.change(input, { target: { files: [file] } });
    fireEvent.change(screen.getByLabelText("Optional background"), { target: { value: "PBMC pilot" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue in Project chat" }));
    await waitFor(() => expect(mocks.registerProjectFiles).toHaveBeenCalledWith("project-1", [file], "PBMC pilot"));
    expect(window.sessionStorage.getItem("shennong.project-handoff:project-1")).toBe("Inspect project://current/resources/resource-1");
    expect(mocks.push).toHaveBeenCalledWith("/projects/project-1/chat");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["projects", "project-1", "context-pack"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["projects", "project-1", "resources"] });
  });
});
