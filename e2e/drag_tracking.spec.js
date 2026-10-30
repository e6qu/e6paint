// Drag-tracking e2e: a moved element must follow the mouse cursor exactly, at
// devicePixelRatio 1 and 2 (Retina). This pins the family of "element jumps
// ahead of the cursor" bugs — the alignment-snap delta was once double-added
// by the drag gesture, which only a real browser drag can surface.
//
// Observable: the relay logs the room's element geometry (DOCDBG) after every
// imported batch, so the assertion reads exact world coordinates without any
// downloads or camera knowledge: a reference rect drawn at a known screen size
// converts css px to world units. The reference must stay put while the probe
// moves by exactly the cursor delta.

const fs = require("fs");
const os = require("os");
const { test, expect } = require("@playwright/test");
const { collector, waitPainted } = require("./helpers");

const RELAY_LOG = "target/e2e-relay.log";
const modKey = os.platform() === "darwin" ? "Meta" : "Control";

// Headed on purpose: the headless shell coalesces pointer events and stalls
// its frame loop for ~30s, which truncates gestures and delays commits. CI
// wraps this run in xvfb (see .github/workflows/ci.yml).
const test_fixed = test.extend({
  browser: async ({ playwright }, use) => {
    const instance = await playwright.chromium.launch({
      args: ["--enable-unsafe-webgpu"],
      headless: false,
    });
    await use(instance);
    await instance.close();
  },
});

function latestDocRects(roomId) {
  try {
    const lines = fs.readFileSync(RELAY_LOG, "utf8").split("\n");
    for (let i = lines.length - 1; i >= 0; i--) {
      const marker = `DOCDBG room=${roomId} `;
      const at = lines[i].indexOf(marker);
      if (at >= 0) {
        const parsed = JSON.parse(lines[i].slice(at + marker.length));
        const rects = [];
        for (const value of Object.values(parsed)) {
          if (value.kind === "rect")
            rects.push({ x: value.x, y: value.y, w: value.w, h: value.h });
        }
        return rects;
      }
    }
  } catch {
    // The log may not exist yet.
  }
  return [];
}

async function waitForDocRects(page, roomId, count, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const rects = latestDocRects(roomId);
    if (rects.length >= count) return rects;
    await page.waitForTimeout(300);
  }
  return [];
}

for (const dsf of [1, 2]) {
  test_fixed(`moved element tracks the cursor at deviceScaleFactor ${dsf}`, async ({
    browser,
    request,
  }) => {
    test.setTimeout(180_000);
    const context = await browser.newContext({ deviceScaleFactor: dsf });
    const page = await context.newPage();
    const lines = collector(page);
    const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
    await page.goto(`/?room=${room.room_id}&token=${room.token}`);
    await waitPainted(page, lines);

    // The join snapshot refits the camera; draw only after it has landed, so
    // the calibration and the drag share one camera epoch.
    await expect
      .poll(() => lines.some((t) => t.includes("board snapshot applied")), {
        timeout: 30_000,
      })
      .toBe(true);
    await page.waitForTimeout(1_000);

    const canvas = page.locator("#e6paint_canvas");
    const box = await canvas.boundingBox();
    // Keyboard presses must follow a canvas click: drawer re-renders on
    // selection changes can steal focus, and a hotkey sent elsewhere is
    // silently lost. Each click lands on a different spot — two clicks on
    // one spot is a double-click, which drops a sticky.
    let clickSpot = 0;
    async function pressKey(key) {
      clickSpot += 1;
      await page.mouse.click(box.x + 60 + clickSpot * 12, box.y + 45);
      await page.waitForTimeout(150);
      await page.keyboard.press(key);
      await page.waitForTimeout(300);
    }

    // Reference rect: 60x40 css px — converts css px to world units.
    await pressKey("4");
    await page.mouse.move(box.x + 300, box.y + 300);
    await page.mouse.down();
    await page.mouse.move(box.x + 360, box.y + 340, { steps: 4 });
    await page.mouse.up();
    const afterRef = await waitForDocRects(page, room.room_id, 1);
    expect(afterRef.length, "reference rect never reached the room").toBe(1);

    // Probe rect: 120x80 css px, placed away from the reference. Rect draws
    // auto-switch back to Select, so re-arm the tool.
    const px = box.x + 700, py = box.y + 500;
    await pressKey("4");
    await page.mouse.move(px, py);
    await page.mouse.down();
    await page.mouse.move(px + 120, py + 80, { steps: 6 });
    await page.mouse.up();
    const before = await waitForDocRects(page, room.room_id, 2);
    expect(before.length, `expected 2 rects: ${before.length}`).toBe(2);
    // The reference is the leftmost rect (drawn left of the probe; the camera
    // can scale positions but never reorder them horizontally).
    const ref1 = before.reduce((a, b) => (b.x < a.x ? b : a));
    const probe1 = before.find((r) => r !== ref1);
    const scaleX = ref1.w / 60;
    const scaleY = ref1.h / 40;
    expect(scaleX, "reference rect missing from the room doc").toBeGreaterThan(0);

    // Select tool; drag the probe rect by exactly (60, 40) css px.
    await pressKey("1");
    const cx = px + 60, cy = py + 40;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 60, cy + 40, { steps: 12 });
    await page.mouse.up();
    let after = [];
    const dragDeadline = Date.now() + 15_000;
    while (Date.now() < dragDeadline) {
      after = latestDocRects(room.room_id);
      if (
        after.length === 2 &&
        after.some((r) => Math.abs(r.w - probe1.w) < 0.5 && Math.abs(r.x - probe1.x) > 2)
      )
        break;
      await page.waitForTimeout(300);
    }
    const ref2 = after.find(
      (r) => Math.abs(r.w - ref1.w) < 0.5 && Math.abs(r.h - ref1.h) < 0.5
    );
    const probe2 = after.find((r) => r !== ref2);
    expect(ref2, "reference rect missing after the drag").toBeTruthy();
    expect(probe2, "probe rect missing after the drag").toBeTruthy();

    // The reference must not have moved (guards against global shifts).
    expect(Math.abs(ref2.x - ref1.x)).toBeLessThan(2);
    expect(Math.abs(ref2.y - ref1.y)).toBeLessThan(2);

    // The probe must have moved by exactly the cursor delta, in world units.
    // Tolerance covers measurement noise plus a legitimate alignment-snap
    // stick (bounded by the 6-px threshold); the historical bug doubled the
    // delta, which is far outside this band.
    const wantDx = 60 * scaleX;
    const wantDy = 40 * scaleY;
    const dx = probe2.x - probe1.x;
    const dy = probe2.y - probe1.y;
    expect(
      Math.abs(dx - wantDx),
      `x delta ${dx.toFixed(1)} vs ${wantDx.toFixed(1)}`
    ).toBeLessThan(8);
    expect(
      Math.abs(dy - wantDy),
      `y delta ${dy.toFixed(1)} vs ${wantDy.toFixed(1)}`
    ).toBeLessThan(8);
  });
}
