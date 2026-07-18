import { expect, test, type Page } from "@playwright/test";

const email = process.env.SHENNONG_E2E_EMAIL;
const password = process.env.SHENNONG_E2E_PASSWORD;

async function signIn(page: Page) {
  test.skip(!email || !password, "SHENNONG_E2E_EMAIL and SHENNONG_E2E_PASSWORD are required");
  await page.goto("/auth/sign-in");
  await page.getByLabel("Email").fill(email!);
  await page.getByLabel("Password").fill(password!);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByText("Your secure session is active.")).toBeVisible();
}

test("project observations persist into a bounded BioGraph", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("response", (response) => {
    if (response.url().includes("/api/v1/") && response.status() >= 500) {
      errors.push(`${response.status()} ${response.url()}`);
    }
  });

  await signIn(page);
  await page.goto("/projects");
  await expect(page.getByRole("heading", { name: "Projects", exact: true })).toBeVisible();

  const projectName = `BioGraph QA ${testInfo.project.name} ${Date.now()}`;
  await page.getByRole("button", { name: "New project" }).first().click();
  await page.getByLabel("Project name").fill(projectName);
  await page.getByLabel("Description").fill("Live browser verification for the Research Graph");
  await page.getByRole("button", { name: "Create project" }).click();
  await expect(page).toHaveURL(/\/projects\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { name: projectName })).toBeVisible();

  const projectId = new URL(page.url()).pathname.split("/").filter(Boolean).at(-1)!;
  const sampleId = `sample-${testInfo.project.name}-${Date.now()}`;
  await page.evaluate(async ({ projectId, sampleId }) => {
    const response = await fetch(`/api/v1/projects/${encodeURIComponent(projectId)}/entities`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        id: sampleId,
        project_id: projectId,
        category: "sample",
        kind: "biospecimen",
        label: `QA biospecimen ${sampleId}`,
        metadata: { source: "playwright" },
        provenance: { actor_type: "user", interface: "browser_qa" },
      }),
    });
    if (!response.ok) throw new Error(`sample setup failed: ${response.status} ${await response.text()}`);
  }, { projectId, sampleId });

  await page.reload();
  await expect(page.getByText(`QA biospecimen ${sampleId}`)).toBeVisible();
  await page.getByLabel("Row 1 sample or entity ID").fill(sampleId);
  await page.getByLabel("Row 1 measurement type").fill("qpcr_ct");
  await page.getByLabel("Row 1 value").fill("21.4");
  await page.getByLabel("Row 1 unit").fill("Ct");
  const recordObservations = page.getByRole("button", { name: "Record observations" });
  await recordObservations.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByText("Observation batch persisted with evidence.")).toBeVisible();
  await expect(page.getByText(`${sampleId} · qpcr_ct`)).toBeVisible();

  await page.getByRole("navigation", { name: "Project sections" }).getByRole("link", { name: "BioGraph" }).click();
  await expect(page.getByRole("heading", { name: "Focused subgraph" })).toBeVisible();
  await expect(page.getByText(/2 nodes · 1 paths/)).toBeVisible();
  const returnedPath = page.locator(".graph-path-list > button").first();
  await expect(returnedPath).toBeVisible();
  await expect(returnedPath).toContainText("shennong:has_observation");
  await expect(page.locator(".project-graph-canvas")).toBeVisible();
  await page.screenshot({ path: `/tmp/shennong-biograph-qa/${testInfo.project.name}.png`, fullPage: true });
  expect(errors).toEqual([]);
});

test("project upload streams a real file and returns the bound Resource", async ({ page }, testInfo) => {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("response", (response) => {
    if (response.url().includes("/api/v1/") && response.status() >= 500) {
      errors.push(`${response.status()} ${response.url()}`);
    }
  });

  await signIn(page);
  await page.goto("/projects");
  const suffix = `${testInfo.project.name}-${Date.now()}`;
  const projectName = `Upload QA ${suffix}`;
  await page.getByRole("button", { name: "New project" }).first().click();
  await page.getByLabel("Project name").fill(projectName);
  await page.getByLabel("Description").fill("Live browser verification for Project-scoped uploads");
  await page.getByRole("button", { name: "Create project" }).click();
  await expect(page).toHaveURL(/\/projects\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { name: projectName })).toBeVisible();

  await page.getByRole("link", { name: "Upload data" }).click();
  await expect(page.getByRole("heading", { name: "Select files" })).toBeVisible();
  await page.locator('input[type="file"]').setInputFiles({
    name: `expression-${suffix}.tsv`,
    mimeType: "text/tab-separated-values",
    buffer: Buffer.from("gene\tsample_a\nTP53\t12\n", "utf8"),
  });
  await page.getByRole("button", { name: "Continue" }).click();

  const resourceId = `upload-qa-${suffix}`;
  const resourceName = `Expression upload ${suffix}`;
  await page.getByLabel("Resource ID").fill(resourceId);
  await page.getByLabel("Resource name").fill(resourceName);
  await page.getByLabel("Description").fill("Browser-uploaded TSV fixture");
  await page.getByLabel("Organism").fill("Homo sapiens");
  await page.getByLabel("Modality").fill("bulk RNA-seq");
  await page.getByRole("button", { name: "Continue" }).click();

  await page.getByLabel("Artifact format").fill("tsv");
  await page.getByRole("button", { name: "Continue" }).click();
  await page.locator("form.upload-card").evaluate((form: HTMLFormElement) => form.requestSubmit());
  await expect(page.getByRole("status")).toContainText(`Resource ${resourceId} registered successfully`);
  await page.getByRole("button", { name: "Return to project" }).click();
  await expect(page.getByText(resourceName)).toBeVisible();
  expect(errors).toEqual([]);
});
