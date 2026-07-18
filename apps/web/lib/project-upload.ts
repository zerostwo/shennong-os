import { registerProjectUploads, uploadProjectFile, type JsonRecord } from "@/lib/api/adapter";
import { randomUuid } from "@/lib/random-uuid";

const FORMAT_BY_EXTENSION: Record<string, string> = {
  csv: "csv",
  h5: "hdf5",
  h5ad: "h5ad",
  json: "json",
  loom: "loom",
  mtx: "matrix-market",
  parquet: "parquet",
  qs: "qs",
  qs2: "qs2",
  rds: "rds",
  tsv: "tsv",
  txt: "text",
  xlsx: "xlsx",
};

export type ProjectUploadResult = {
  resource: JsonRecord;
  resourceId: string;
  resourceName: string;
  uri: string;
  filenames: string[];
};

function extension(filename: string) {
  return filename.toLowerCase().split(".").at(-1) ?? "";
}

function displayName(filename: string) {
  const withoutExtension = filename.replace(/\.[^.]+$/, "");
  const readable = withoutExtension.replace(/[._-]+/g, " ").replace(/\s+/g, " ").trim();
  return readable || "Uploaded dataset";
}

function safeId(value: string) {
  const normalized = value
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[._-]+|[._-]+$/g, "")
    .slice(0, 120);
  return normalized || "uploaded-dataset";
}

export function inferProjectUploadRegistration(files: readonly File[], background = "") {
  if (files.length === 0) throw new Error("Select at least one file.");
  if (files.length > 20) throw new Error("Upload no more than 20 related files at once.");
  const resourceName = files.length === 1 ? displayName(files[0].name) : `${displayName(files[0].name)} and related files`;
  const formats = new Set(files.map((file) => FORMAT_BY_EXTENSION[extension(file.name)] ?? "binary"));
  const resourceId = `${safeId(resourceName)}-${randomUuid().slice(0, 8)}`;
  const filenames = files.map((file) => file.name);
  const context = background.trim().slice(0, 4000);
  return {
    resource_id: resourceId,
    name: resourceName,
    description: context || `Uploaded for Agent-led organization. Files: ${filenames.join(", ")}`,
    format: formats.size === 1 ? [...formats][0] : "binary",
    data_class: "raw",
    visibility: "private",
  } satisfies JsonRecord;
}

export async function registerProjectFiles(projectId: string, files: readonly File[], background = ""): Promise<ProjectUploadResult> {
  const registration = inferProjectUploadRegistration(files, background);
  const uploads: JsonRecord[] = [];
  for (const file of files) uploads.push(await uploadProjectFile(projectId, file));
  const resource = await registerProjectUploads(projectId, {
    ...registration,
    upload_ids: uploads.map((upload) => String(upload.id)),
  });
  const resourceId = String(resource.id ?? registration.resource_id);
  return {
    resource,
    resourceId,
    resourceName: String(resource.name ?? registration.name),
    uri: `project://current/resources/${resourceId}`,
    filenames: files.map((file) => file.name),
  };
}

export function buildProjectUploadPrompt(result: ProjectUploadResult, background = "") {
  const context = background.trim().slice(0, 4000);
  const shownFiles = result.filenames.slice(0, 20);
  return [
    context || "Please inspect and organize the uploaded experimental data.",
    "",
    "Uploaded Project resource:",
    `- ${result.resourceName}: ${result.uri}`,
    `- Files: ${shownFiles.join(", ")}`,
    "",
    "Infer metadata that is supported by the files and this Project context. Ask only for background that cannot be inferred, then organize the resource for analysis.",
  ].join("\n");
}
