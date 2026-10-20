// Throwaway: room-first key delivery probe.
const { test } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

test("room-first key delivery probe", async ({ page, request }) => {
  const lines = collector(page);
  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  await page.goto(`/?room=${room.room_id}&token=${room.token}`);
  await waitPainted(page, lines);
  await page.evaluate(() => {
    window.__keys = [];
    for (const type of ["keydown"]) {
      window.addEventListener(
        type,
        (e) => {
          window.__keys.push(
            `keydown key=${e.key} ctrl=${e.ctrlKey} target=${e.target.tagName}#${e.target.id} defaultPrevented=${e.defaultPrevented}`
          );
        },
        true
      );
    }
  });
  await page.locator("#e6paint_canvas").focus();
  await page.keyboard.press("Control+e");
  await page.waitForTimeout(500);
  const keys = await page.evaluate(() => window.__keys);
  const active = await page.evaluate(() => {
    const el = document.activeElement;
    return el ? `${el.tagName}#${el.id}` : "none";
  });
  console.log("KEYS:\n" + keys.join("\n") + "\nACTIVE ELEMENT: " + active);
  console.log("APP:\n" + lines.join("\n"));
});
