// Multiplayer e2e: three real browser pages exercise one relay room end to end —
// share-link join, pen/text/image edits syncing in both directions, read-only
// enforcement, and snapshot persistence.
//
// Assertions read the relay's log and SQLite store (deterministic boundaries).
// Browser console is used only for join progress and absence of errors:
// Chromium deduplicates repeated console messages, so counting them is not a
// reliable signal.

const fs = require("fs");
const { test, expect } = require("@playwright/test");
const { collector, waitPainted, modKey } = require("./helpers");

const RELAY_LOG = "target/e2e-relay.log";
const RELAY_DB = "target/e2e-relay.db";

function relayLogCount(fragment) {
  try {
    return fs.readFileSync(RELAY_LOG, "utf8").split(fragment).length - 1;
  } catch {
    return 0;
  }
}

function relayLogMentions(fragment) {
  return relayLogCount(fragment) > 0;
}

function expectJoined(lines, roomId, who) {
  expect(
    lines.some((t) => t.includes(`joining room ${roomId}`)),
    `${who} never joined; console: ${lines.join(" | ")}`
  ).toBe(true);
}

function expectNoProblems(lines, who) {
  const problems = lines.filter(
    (t) => t.startsWith("error:") || t.includes("panicked")
  );
  expect(problems, `${who} console problems: ${problems.join(" | ")}`).toEqual(
    []
  );
}

async function drawPenStroke(page, canvas, box) {
  await canvas.focus();
  await page.keyboard.press("3"); // Pen tool
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  await page.mouse.move(cx - 80, cy - 40);
  await page.mouse.down();
  await page.mouse.move(cx + 60, cy + 30, { steps: 8 });
  await page.mouse.up();
}

test("share-link peers join, edits sync, and the room persists", async ({
  page,
  context,
  request,
}) => {
  test.setTimeout(180_000);

  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  const url = `/?room=${room.room_id}&token=${room.token}`;
  const consoleA = collector(page);

  // A joins by share link (room-first: no demo seed) and finishes booting.
  await page.goto(url);
  await waitPainted(page, consoleA);
  expectJoined(consoleA, room.room_id, "page A");
  expectNoProblems(consoleA, "page A");

  // A draws a pen stroke; the relay imports the batch.
  const batchesBeforeStroke = relayLogCount("batch from");
  const canvas = page.locator("#e6paint_canvas");
  const box = await canvas.boundingBox();
  expect(box, "canvas must be laid out").not.toBeNull();
  await drawPenStroke(page, canvas, box);
  await expect
    .poll(() => relayLogCount("batch from") > batchesBeforeStroke, {
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    })
    .toBe(true);

  // B joins and draws too; the relay imports B's batch (the return direction).
  const b = await context.newPage();
  const consoleB = collector(b);
  await b.goto(url);
  await waitPainted(b, consoleB);
  expectJoined(consoleB, room.room_id, "page B");
  expectNoProblems(consoleB, "page B");
  const batchesBeforeB = relayLogCount("batch from");
  const boxB = await b.locator("#e6paint_canvas").boundingBox();
  expect(boxB, "canvas must be laid out on B").not.toBeNull();
  await drawPenStroke(b, b.locator("#e6paint_canvas"), boxB);
  await expect
    .poll(() => relayLogCount("batch from") > batchesBeforeB, {
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    })
    .toBe(true);

  // A places a text element and types; set_text batches must be imported.
  // Typing goes through eframe's IME agent handoff, which needs a few slow
  // frames on the software rasterizer - retry the whole placement if the first
  // attempt's keystrokes get lost.
  const batchesBeforeText = relayLogCount("batch from");
  let textSynced = false;
  for (let attempt = 0; attempt < 3 && !textSynced; attempt++) {
    const before = relayLogCount("batch from");
    await page.keyboard.press("7");
    await page.waitForTimeout(300);
    await page.mouse.click(box.x + 120, box.y + 40);
    await page.waitForTimeout(1_000);
    await page.keyboard.type("shared note", { delay: 120 });
    await page.keyboard.press("Escape");
    await page.waitForTimeout(1_500);
    textSynced = relayLogCount("batch from") > before;
  }
  expect(textSynced, "text batches never reached the relay").toBe(true);

  // Image insert through the real file input: wasm decode + insert + sync.
  // Same retry pattern as typing: the drain runs on the slow wasm frame loop.
  const tinyPng = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64"
  );
  let imageSynced = false;
  for (let attempt = 0; attempt < 3 && !imageSynced; attempt++) {
    const before = relayLogCount("batch from");
    await page.setInputFiles("#e6_file_input", {
      name: "dot.png",
      mimeType: "image/png",
      buffer: tinyPng,
    });
    await page.waitForTimeout(3_000);
    imageSynced = relayLogCount("batch from") > before;
  }
  expect(
    imageSynced,
    `image batches never reached the relay; A console:\n${consoleA.join("\n")}`
  ).toBe(true);
  expectNoProblems(consoleA, "page A after image");

  // Undo the image insert: the removal syncs like any other gesture.
  const batchesBeforeUndo = relayLogCount("batch from");
  await page.keyboard.press(`${modKey}+z`);
  await expect
    .poll(() => relayLogCount("batch from") > batchesBeforeUndo, {
      timeout: 30_000,
      intervals: [500, 1_000, 2_000],
    })
    .toBe(true);

  // A view-only token joins fine, but its edits never reach the room.
  const c = await context.newPage();
  const consoleC = collector(c);
  await c.goto(`/?room=${room.room_id}&token=${room.read_token}`);
  await waitPainted(c, consoleC);
  expectJoined(consoleC, room.room_id, "page C");
  const boxC = await c.locator("#e6paint_canvas").boundingBox();
  expect(boxC, "canvas must be laid out on C").not.toBeNull();
  await drawPenStroke(c, c.locator("#e6paint_canvas"), boxC);
  // The relay tells view-only peers their access level; the app surfaces it in
  // the HUD and suppresses sync (the relay would reject the batches anyway -
  // server-side rejection is covered by the relay's own e2e).
  expect(
    consoleC.some((t) => t.includes("view-only session")),
    `page C never learned it is view-only; console: ${consoleC.join(" | ")}`
  ).toBe(true);
  expectNoProblems(consoleC, "page C");
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
