import { describe, expect, it, vi } from "vitest";
import { buildProjectUploadPrompt, inferProjectUploadRegistration } from "./project-upload";

vi.mock("./random-uuid", () => ({ randomUuid: () => "12345678-1234-1234-1234-123456789012" }));

describe("project upload helpers", () => {
  it("infers only safe minimal registration metadata", () => {
    const result = inferProjectUploadRegistration([new File(["x"], "PBMC counts.tsv")], "10x PBMC pilot");
    expect(result).toMatchObject({
      resource_id: "pbmc-counts-12345678",
      name: "PBMC counts",
      description: "10x PBMC pilot",
      format: "tsv",
      data_class: "raw",
      visibility: "private",
    });
    expect(result).not.toHaveProperty("project_id");
    expect(result).not.toHaveProperty("owner_user_id");
  });

  it("builds a durable Project resource handoff for the Agent", () => {
    expect(buildProjectUploadPrompt({ resource: {}, resourceId: "pbmc-1", resourceName: "PBMC", uri: "project://current/resources/pbmc-1", filenames: ["matrix.tsv"] }))
      .toContain("project://current/resources/pbmc-1");
  });
});
