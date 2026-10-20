// Export e2e: cmd/ctrl+E (SVG) and cmd/ctrl+shift+E (PNG) download real files
// from the live wasm app — including the resvg text-rasterization path on wasm,
// which has no system fonts to lean on.

const fs = require("fs");
const os = require("os");
const { test, expect } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

const isMac = os.platform() === "darwin";
const modKey = isMac ? "Meta" : "Control";

test("export shortcuts download valid SVG and PNG", async ({ page }) => {
  test.setTimeout(120_000);
  const lines = collector(page);
  await page.goto("/");
  await waitPainted(page, lines);

  // Give the board content: a rect and a text element (text exercises resvg's
  // font path on wasm).
  await page.keyboard.press("4");
  const canvas = page.locator("#e6paint_canvas");
  const box = await canvas.boundingBox();
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx - 120, cy - 80);
  await page.mouse.down();
  await page.mouse.move(cx + 40, cy + 40, { steps: 8 });
  await page.mouse.up();
  await page.keyboard.press("7");
  await page.mouse.click(cx + 120, cy - 40);
  await page.waitForTimeout(400);
  await page.keyboard.type("export me");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(500);

  // SVG export.
  const svgDownload = page.waitForEvent("download", { timeout: 20_000 });
  await page.keyboard.press(`${modKey}+e`);
  const svg = await svgDownload;
  expect(svg.suggestedFilename()).toMatch(/\.svg$/);
  const svgPath = await svg.path();
  const svgText = fs.readFileSync(svgPath, "utf8");
  expect(svgText.startsWith("<svg")).toBe(true);
  expect(svgText).toContain("export me");

  // PNG export (resvg on wasm).
  const pngDownload = page.waitForEvent("download", { timeout: 30_000 });
  await page.keyboard.press(`${modKey}+Shift+e`);
  const png = await pngDownload;
  expect(png.suggestedFilename()).toMatch(/\.png$/);
  const pngPath = await png.path();
  const pngBytes = fs.readFileSync(pngPath);
  expect(pngBytes.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))).toBe(true);
  expect(pngBytes.length).toBeGreaterThan(1_000);

  // No export or edit failures in the console.
  const problems = lines.filter(
    (t) =>
      t.includes("failed") ||
      t.includes("panicked") ||
      t.startsWith("error:")
  );
  expect(problems, `console problems: ${problems.join(" | ")}`).toEqual([]);
});
