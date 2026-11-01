// PNG rasterization worker: receives { svg, scale } jobs, replies with
// { png: Uint8Array } or { error }. Built by the trunk pre-build hook
// (render_worker.js + render_worker_bg.wasm land in static/worker/).
importScripts('./render_worker.js');

let ready = false;
const queue = [];

const handle = (job) => {
  try {
    const png = wasm_bindgen.rasterize_svg_png(job.svg, job.scale);
    self.postMessage({ png }, [png.buffer]);
  } catch (err) {
    self.postMessage({ error: String(err) });
  }
};

wasm_bindgen('./render_worker_bg.wasm')
  .then(() => {
    ready = true;
    for (const job of queue.splice(0)) {
      handle(job);
    }
  })
  .catch((err) => {
    self.postMessage({ error: 'worker init failed: ' + err });
  });

self.onmessage = (event) => {
  if (ready) {
    handle(event.data);
  } else {
    // Jobs arriving before the wasm finishes initializing.
    queue.push(event.data);
  }
};
