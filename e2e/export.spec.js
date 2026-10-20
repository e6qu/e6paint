// Export e2e: cmd/ctrl+E (SVG) and cmd/ctrl+shift+E (PNG) download real files
// from the live wasm app — including the resvg text-rasterization path on wasm,
// which has no system fonts to lean on.
//
// Two separate tests: Chromium throttles multiple automatic downloads in one
// context, so each export gets a fresh page.

const fs = require("fs");
const os = require("os");
const base = require("@playwright/test");
const { expect } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

const isMac = os.platform() === "darwin";
const modKey = isMac ? "Meta" : "Control";

// Chromium throttles automatic downloads per browser process (a couple per
// time window). Each export test gets a fresh browser so the budget resets.
const test = base.test.extend({
  browser: async ({ playwright }, use) => {
    const instance = await playwright.chromium.launch({
      args: ["--enable-unsafe-webgpu"],
    });
    await use(instance);
    await instance.close();
  },
});

async function seedBoard(page, url) {
  const lines = collector(page);
  await page.goto(url);
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
  await page.waitForTimeout(700);
  // Slow typing: each key lands in its own frame, so every set_text applies.
  await page.keyboard.type("export me", { delay: 60 });
  await page.keyboard.press("Escape");
  await page.waitForTimeout(800);
  return lines;
}

// Room-first empty sessions: the demo board's 2,500 elements would make the
// synchronous wasm PNG raster freeze the page for ~30 s (the parked §14 worker
// offload). Two elements isolate the export path itself.
test("cmd/ctrl+E downloads valid SVG", async ({ page, request }) => {
  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  const lines = await seedBoard(
    page,
    `/?room=${room.room_id}&token=${room.token}`
  );

  // The inline editor's keystroke sync lands over consecutive frames; retry the
  // export until the downloaded SVG reflects the typed text.
  let svgText = String();
  for (let attempt = 0; attempt < 5; attempt++) {
    const download = page.waitForEvent("download", { timeout: 15_000 });
    await page.keyboard.press(`${modKey}+e`);
    const file = await download;
    expect(file.suggestedFilename()).toMatch(/\.svg$/);
    svgText = fs.readFileSync(await file.path(), "utf8");
    if (svgText.includes("export me")) break;
    await page.waitForTimeout(1_500);
  }
  expect(svgText.startsWith("<svg")).toBe(true);
  expect(svgText).toContain("export me");

  const problems = lines.filter(
    (t) => t.includes("failed") || t.includes("panicked") || t.startsWith("error:")
  );
  expect(problems, `console problems: ${problems.join(" | ")}`).toEqual([]);
});

test("cmd/ctrl+shift+E downloads valid PNG", async ({ page, request }) => {
  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  const lines = await seedBoard(
    page,
    `/?room=${room.room_id}&token=${room.token}`
  );

  const download = page.waitForEvent("download", { timeout: 30_000 });
  await page.keyboard.press("Control+Shift+e");
  const file = await download.catch(async (err) => {
    console.log("PNG EXPORT TIMEOUT; app console:\n" + lines.join("\n"));
    throw err;
  });
  expect(file.suggestedFilename()).toMatch(/\.png$/);
  const bytes = fs.readFileSync(await file.path());
  expect(
    bytes
      .subarray(0, 8)
      .equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))
  ).toBe(true);
  expect(bytes.length).toBeGreaterThan(1_000);

  const problems = lines.filter(
    (t) => t.includes("failed") || t.includes("panicked") || t.startsWith("error:")
  );
  expect(problems, `console problems: ${problems.join(" | ")}`).toEqual([]);
});
