// Smoke e2e: the wasm app boots against the real page, announces its first
// painted frame, and never hits its loud startup failures.

const { test, expect } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

test("app boots, paints the canvas, and stays quiet in the console", async ({
  page,
}) => {
  const consoleLines = collector(page);
  page.on("pageerror", (err) => consoleLines.push(`pageerror: ${err}`));

  await page.goto("/");
  const canvas = page.locator("#e6paint_canvas");
  await expect(canvas).toBeVisible();

  // The app logs this only after egui has actually run a frame.
  await waitPainted(page, consoleLines);

  // The app fails loudly by contract; any of these in the console is a defect.
  const problems = consoleLines.filter(
    (t) =>
      t.startsWith("error:") ||
      t.includes("panicked") ||
      t.includes("refusing to start") ||
      t.includes("WebGPU")
  );
  expect(problems, `console problems: ${problems.join(" | ")}`).toEqual([]);
});
