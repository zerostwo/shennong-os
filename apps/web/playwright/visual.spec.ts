import { mkdir } from "node:fs/promises";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";

const configuredOutput = process.env.SHENNONG_VISUAL_OUTPUT_DIR;
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

async function screenshotPath(testInfo: { outputDir: string; project: { name: string } }, surface: string) {
  const output = configuredOutput ? path.resolve(configuredOutput) : testInfo.outputDir;
  await mkdir(output, { recursive: true });
  const viewport = testInfo.project.name === "chromium" ? "desktop-1512x801" : "mobile-pixel5";
  return path.join(output, `shennong-os-v1-${surface}-${viewport}.png`);
}

test.beforeEach(async ({ page }, testInfo) => {
  if (testInfo.project.name === "chromium") {
    await page.setViewportSize({ width: 1512, height: 801 });
  }
});

test("capture personal-chat home", async ({ page }, testInfo) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "What can I help you analyze?" })).toBeVisible();
  await expect(page.getByPlaceholder("Ask Shennong")).toBeVisible();
  await page.screenshot({ path: await screenshotPath(testInfo, "home"), fullPage: false });
});

test("capture centered Search dialog", async ({ page }, testInfo) => {
  await page.goto("/");
  const mobileMenu = page.getByRole("button", { name: "Open navigation" });
  if (await mobileMenu.isVisible()) await mobileMenu.click();
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.screenshot({ path: await screenshotPath(testInfo, "search"), fullPage: false });
});

test("capture the Project-bound Agent workspace", async ({ page }, testInfo) => {
  const apiErrors: string[] = [];
  page.on("response", (response) => {
    const pathname = new URL(response.url()).pathname;
    const isEmptyThreadProbe = response.status() === 404
      && /^\/api\/v1\/threads\/[0-9a-f-]{36}\/(?:messages|runs\/active)$/.test(pathname);
    if (pathname.includes("/api/v1/") && response.status() >= 400 && !isEmptyThreadProbe) {
      apiErrors.push(`${response.status()} ${pathname}`);
    }
  });
  await signIn(page);
  await page.goto("/projects");
  const projectName = `Visual Agent QA ${testInfo.project.name} ${Date.now()}`;
  await page.getByRole("button", { name: "New project" }).first().click();
  await page.getByLabel("Project name").fill(projectName);
  await page.getByLabel("Description").fill("Stable visual fixture for the Shennong OS Agent workspace");
  await page.getByRole("button", { name: "Create project" }).click();
  await expect(page).toHaveURL(/\/projects\/[0-9a-f-]{36}$/);
  const projectId = new URL(page.url()).pathname.split("/").filter(Boolean).at(-1)!;
  await page.goto(`/projects/${encodeURIComponent(projectId)}/chat`);
  await expect(page.getByRole("heading", { name: "What can I help you analyze?" })).toBeVisible();
  await expect(page.getByText("Generated code runs in an isolated Runtime.")).toBeVisible();
  expect(apiErrors).toEqual([]);
  await page.screenshot({ path: await screenshotPath(testInfo, "agent"), fullPage: false });
});
