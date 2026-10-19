// Shared e2e helpers.

const { expect } = require("@playwright/test");

/**
 * Wait until the app announces its first painted frame ("first frame painted"
 * at info level). Debug-wasm boots can take tens of seconds; fixed sleeps are
 * worthless against that.
 */
async function waitPainted(page, collector) {
  await expect
    .poll(() => collector.some((t) => t.includes("first frame painted")), {
      timeout: 60_000,
      intervals: [250, 500, 1_000, 2_000],
    })
    .toBe(true);
}

/** Per-page console collector (info + error lines). */
function collector(page) {
  const lines = [];
  page.on("console", (msg) => {
    if (msg.type() === "info" || msg.type() === "error") lines.push(msg.text());
  });
  return lines;
}

module.exports = { waitPainted, collector };
