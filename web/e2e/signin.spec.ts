import { expect, test, type Page } from "@playwright/test";
import { existsSync } from "node:fs";
import { demoData, emptyData, setupCode, startServer, type Server } from "./server";

/** Every request the page makes must go to the app itself. */
function recordOrigins(page: Page, app: string): string[] {
  const others: string[] = [];
  page.on("request", (r) => {
    const u = new URL(r.url());
    if (u.protocol === "data:" || u.protocol === "blob:") return;
    if (u.origin !== app) others.push(u.origin);
  });
  return others;
}

test.describe("first run", () => {
  let server: Server;
  test.beforeEach(async () => {
    server = await startServer({ data: emptyData(), cleanup: true });
  });
  test.afterEach(async () => server.stop());

  test("set the password with the setup code, then sign out and in", async ({ page }) => {
    const others = recordOrigins(page, server.url);
    await page.goto(server.url);
    await expect(page.getByRole("heading", { name: "Set a password" })).toBeVisible();
    // Plain HTTP on a non-localhost name would show the notice; 127.0.0.1 is
    // a secure context, so it must not block anything either way.
    await page.getByLabel("Setup code").fill("WRNG-CODE");
    await page.getByLabel("Password", { exact: true }).fill("a fine password");
    await page.getByLabel("Password again").fill("a fine password");
    await page.getByRole("button", { name: "Set password" }).click();
    await expect(page.getByRole("alert")).toContainText("setup code is not right");

    await page.getByLabel("Setup code").fill(setupCode(server.log()).toLowerCase());
    await page.getByRole("button", { name: "Set password" }).click();
    await expect(page.getByTestId("no-archive")).toBeVisible();
    expect(server.log()).not.toContain("a fine password");

    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
    await page.getByLabel("Password").fill("not the password");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByRole("alert")).toHaveText("Wrong password.");
    await page.getByLabel("Password").fill("a fine password");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByTestId("no-archive")).toBeVisible();

    // The session survives a reload.
    await page.reload();
    await expect(page.getByTestId("no-archive")).toBeVisible();
    expect(others).toEqual([]);
  });
});

test.describe("demo archive", () => {
  test.skip(!existsSync(`${demoData}/.demo-ready`), "run scripts/demo.sh --prepare first");
  let server: Server;
  test.beforeEach(async () => {
    server = await startServer({ data: demoData, demo: true });
  });
  test.afterEach(async () => server.stop());

  test("sign in to the demo and see its counts", async ({ page }, info) => {
    const others = recordOrigins(page, server.url);
    await page.goto(server.url);
    await expect(page.getByTestId("demo-notice")).toBeVisible();
    await page.screenshot({ path: info.outputPath(`sign-in-${info.project.name}.png`) });
    await page.getByLabel("Password").fill("demo1234");
    await page.getByRole("button", { name: "Sign in" }).click();
    const counts = page.getByTestId("archive-counts");
    await expect(counts).toBeVisible();
    await expect(counts).toContainText("Messages");
    await expect(counts).toContainText("25,");
    await page.screenshot({ path: info.outputPath(`home-${info.project.name}.png`) });
    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
    expect(others).toEqual([]);
  });
});
