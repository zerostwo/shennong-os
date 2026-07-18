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
  await page.getByRole("link", { name: "Open Admin center" }).click();
  await expect(page).toHaveURL(/\/admin\/invites/);
}

test("the public home enforces a Project boundary and Search opens as a centered dialog", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Choose a research project" })).toBeVisible();
  const mobileMenu = page.getByRole("button", { name: "Open navigation" });
  if (await mobileMenu.isVisible()) await mobileMenu.click();
  await page.getByRole("button", { name: "Search" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByPlaceholder("Search chats, Resources, and Projects")).toBeFocused();
});

test("Agent workspace uses the full viewport whenever the responsive sidebar is hidden", async ({ page }) => {
  for (const width of [761, 800, 900]) {
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeVisible();
    const main = page.locator("main.main-column");
    await expect(main).toHaveCSS("margin-left", "0px");
    await expect(main).toHaveCSS("width", `${width}px`);
  }

  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 800 });
    await page.goto("/");
    const mainBox = await page.locator("main.main-column").boundingBox();
    expect(mainBox?.x).toBe(width === 375 ? 0 : 260);
    expect(mainBox?.width).toBe(width === 375 ? 375 : 1180);
  }
});

test("public Resources are backed by the live API", async ({ page }) => {
  const responses: number[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/api/v1/resources")) responses.push(response.status());
  });
  await page.goto("/resources");
  await expect(page.getByRole("heading", { name: "Resources" })).toBeVisible();
  await expect.poll(() => responses.length).toBeGreaterThan(0);
  expect(responses.every((status) => status < 500)).toBeTruthy();
});

test("authenticated product modules load without browser or API errors", async ({ page }) => {
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
  for (const route of [
    "/",
    "/resources",
    "/projects",
    "/console/jobs",
    "/console/sessions",
    "/admin/invites",
    "/admin/audit",
  ]) {
    await page.goto(route);
    await expect(page.locator("main").getByRole("heading").first()).toBeVisible();
  }
  expect(errors).toEqual([]);
});

test("retired legacy entry points redirect to a supported V1 surface", async ({ page }) => {
  await signIn(page);
  for (const [legacy, supported] of [
    ["/console/my-data", /\/console\/jobs/],
    ["/console/uploads", /\/projects/],
    ["/console/api-access", /\/console\/jobs/],
    ["/admin/dashboard", /\/admin\/invites/],
    ["/admin/settings", /\/admin\/invites/],
    ["/catalog/collections", /\/resources/],
  ] as const) {
    await page.goto(legacy);
    await expect(page).toHaveURL(supported);
  }
});
