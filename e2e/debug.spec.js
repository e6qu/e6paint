// Throwaway: image insert probe.
const { test, request } = require("@playwright/test");
const { waitPainted, collector } = require("./helpers");

test("image insert probe", async ({ page, request }) => {
  const lines = collector(page);
  page.on("console", (msg) => {
    lines.push(msg.type() + ": " + msg.text());
  });
  const room = await (await request.post("http://127.0.0.1:8480/rooms")).json();
  await page.goto(`/?room=${room.room_id}&token=${room.token}`);
  await waitPainted(page, lines);
  lines.push("--- booted; inserting image ---");
  const tinyPng = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64"
  );
  await page.setInputFiles("#e6_file_input", {
    name: "dot.png",
    mimeType: "image/png",
    buffer: tinyPng,
  });
  await page.waitForTimeout(8_000);
  lines.push("--- done; batches check pending ---");
  console.log("DUMP:\n" + lines.join("\n"));
});
