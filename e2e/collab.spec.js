// Multiplayer e2e: two real browser pages join one relay room via share links,
// an edit made on page A flows through the relay, and the room snapshot lands in
// the relay's SQLite store once both pages leave.
//
// Relay observability: the webServer redirects the relay's stderr to
// target/e2e-relay.log (RUST_LOG=info), and the store path is
// target/e2e-relay.db. Both paths are polled from the spec.

const fs = require("fs");
const { test, expect } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

const RELAY_LOG = "target/e2e-relay.log";
const RELAY_DB = "target/e2e-relay.db";

function relayLogMentions(fragment) {
  try {
    return fs.readFileSync(RELAY_LOG, "utf8").includes(fragment);
  } catch {
    return false;
  }
}

test("share-link peers join, an edit syncs, and the room persists", async ({
  page,
  context,
  request,
}) => {
  test.setTimeout(180_000);

  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  const url = `/?room=${room.room_id}&token=${room.token}`;

  const consoleA = collector(page);

  // Page A joins by share link (skips the demo seed) and finishes booting.
  await page.goto(url);
  await waitPainted(page, consoleA);
  expect(
    consoleA.some((t) => t.includes(`joining room ${room.room_id}`)),
    `page A never joined; console: ${consoleA.join(" | ")}`
  ).toBe(true);

  // Pick the Pen tool by its digit (the canvas owns keyboard focus), then drag a
  // stroke. No pre-click: with Select active a click starts a marquee.
  const canvas = page.locator("#e6paint_canvas");
  await canvas.focus();
  await canvas.press("3");
  const box = await canvas.boundingBox();
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx - 80, cy - 40);
  await page.mouse.down();
  await page.mouse.move(cx + 60, cy + 30, { steps: 8 });
  await page.mouse.up();

  // The relay must have imported a batch from the browser peer.
  await expect
    .poll(() => relayLogMentions("batch from"), { timeout: 30_000 })
    .toBe(true);

  // Page B joins the same room and receives A's content from the room.
  const b = await context.newPage();
  const consoleB = collector(b);
  await b.goto(url);
  await waitPainted(b, consoleB);
  expect(
    consoleB.some((t) => t.includes(`joining room ${room.room_id}`)),
    `page B never joined; console: ${consoleB.join(" | ")}`
  ).toBe(true);
  await expect
    .poll(
      () => consoleB.some((t) => t.includes("remote board update applied")),
      { timeout: 15_000, intervals: [500, 1_000, 2_000] }
    )
    .toBe(true);

  // B draws too: the return path (B's batch applied on A) must also hold.
  const boxB = await b.locator("#e6paint_canvas").boundingBox();
  await b.locator("#e6paint_canvas").focus();
  await b.keyboard.press("3");
  await b.mouse.move(boxB.x + 120, boxB.y + 200);
  await b.mouse.down();
  await b.mouse.move(boxB.x + 220, boxB.y + 260, { steps: 8 });
  await b.mouse.up();
  await expect
    .poll(
      () =>
        consoleA.filter((t) => t.includes("remote board update applied"))
          .length >= 1,
      { timeout: 20_000, intervals: [500, 1_000, 2_000] }
    )
    .toBe(true);

  // A places a text element and types into the inline editor; the keystrokes
  // sync to B through the room. Escape closes the editor.
  await page.keyboard.press("7");
  await page.mouse.click(cx + 100, cy + 120);
  await page.waitForTimeout(500);
  await page.keyboard.type("hello from e2e");
  await page.waitForTimeout(1_000);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(1_000);
  expect(
    consoleA.filter((t) => t.includes("text edit apply failed")),
    "inline text editing failed to apply"
  ).toEqual([]);
  await expect
    .poll(
      () =>
        consoleB.filter((t) => t.includes("remote board update applied"))
          .length >= 2,
      { timeout: 20_000, intervals: [500, 1_000, 2_000] }
    )
    .toBe(true);
  expect(
    consoleB.filter((t) => t.startsWith("error:") || t.includes("panicked")),
    `page B console problems: ${consoleB.join(" | ")}`
  ).toEqual([]);

  // A view-only token joins fine, but its edits never reach the room.
  const c = await context.newPage();
  const consoleC = collector(c);
  await c.goto(`/?room=${room.room_id}&token=${room.read_token}`);
  await waitPainted(c, consoleC);
  expect(
    consoleC.some((t) => t.includes(`joining room ${room.room_id}`)),
    `page C never joined; console: ${consoleC.join(" | ")}`
  ).toBe(true);
  const canvasC = c.locator("#e6paint_canvas");
  await canvasC.focus();
  await canvasC.press("3");
  const boxC = await canvasC.boundingBox();
  const ccx = boxC.x + boxC.width / 2;
  const ccy = boxC.y + boxC.height / 2;
  await c.mouse.move(ccx - 80, ccy - 40);
  await c.mouse.down();
  await c.mouse.move(ccx + 60, ccy + 30, { steps: 8 });
  await c.mouse.up();
  await expect
    .poll(
      () => relayLogMentions("batch from read-only peer"),
      { timeout: 20_000 }
    )
    .toBe(true);
  await c.close();

  // Leaving drains the room actor, which persists a snapshot before exiting.
  // The read-only peer's rejected batches must not be part of it.
  await b.close();
  await page.close();
  await expect
    .poll(
      () => {
        try {
          const out = require("child_process").execSync(
            `sqlite3 ${RELAY_DB} "SELECT length(snapshot) FROM rooms WHERE id='${room.room_id}'"`
          );
          return Number(out.toString().trim()) > 0;
        } catch {
          return false;
        }
      },
      { timeout: 45_000, intervals: [1_000, 2_000, 5_000, 10_000] }
    )
    .toBe(true);
});
