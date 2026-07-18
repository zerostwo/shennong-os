import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { UploadView } from "./upload-view";

const mocks=vi.hoisted(()=>({uploadProjectFile:vi.fn(async()=>({id:"upload-1"})),registerProjectUploads:vi.fn(async()=>({id:"resource-1"}))}));
vi.mock("next/navigation", () => ({ usePathname: () => "/projects/project-1/uploads/new", useRouter: () => ({ push: vi.fn() }) }));
vi.mock("@/lib/api/adapter", () => ({ getSession: async () => ({ authenticated:false, user_id:"", role:"", scopes:[] }), getPublicConfig:async()=>({registration_mode:"disabled"}), getHealth:async()=>({status:"ok"}), listIngestionJobs:async()=>[], signOut: async () => undefined, uploadProjectFile:mocks.uploadProjectFile,registerProjectUploads:mocks.registerProjectUploads }));

describe("UploadView", () => {
  it("registers a dataset and invalidates the Project Resource views", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const invalidate = vi.spyOn(client,"invalidateQueries");
    render(<QueryClientProvider client={client}><UploadView projectId="project-1" /></QueryClientProvider>);
    expect(screen.getByRole("heading", { name:"Select files" })).toBeInTheDocument();
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    fireEvent.change(input,{target:{files:[new File(["real"],"matrix.tsv",{type:"text/tab-separated-values"})]}});
    fireEvent.click(screen.getByRole("button", { name:"Continue" }));
    expect(screen.getByRole("heading", { name:"Describe resource" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Resource ID"),{target:{value:"resource-1"}});
    fireEvent.change(screen.getByLabelText("Resource name"),{target:{value:"Resource 1"}});
    fireEvent.click(screen.getByRole("button", { name:"Continue" }));
    await waitFor(()=>expect(screen.getByRole("heading", { name:"Access & format" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name:"Continue" }));
    await waitFor(()=>expect(screen.getByRole("heading", { name:"Upload & register" })).toBeInTheDocument());
    const form=screen.getByRole("button", { name:"Upload and register" }).closest("form")!;
    fireEvent.submit(form);
    fireEvent.submit(form);
    expect(await screen.findByRole("status")).toHaveTextContent("Resource resource-1 registered successfully");
    expect(mocks.uploadProjectFile).toHaveBeenCalledTimes(1);
    expect(mocks.registerProjectUploads).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledWith({queryKey:["projects","project-1","context-pack"]});
    expect(invalidate).toHaveBeenCalledWith({queryKey:["projects","project-1","resources"]});
  });
});
