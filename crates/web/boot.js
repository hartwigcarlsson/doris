// Trunk's initializer (`data-initializer` in index.html): moves the progress
// bar in #boot while the wasm downloads, and removes #boot once the app runs.
export default function initializer() {
  let first;
  return {
    onProgress({ current, total }) {
      if (!total) return;
      const percent = Math.min(100, Math.floor((current / total) * 100));
      const bar = document.querySelector("#boot [role=progressbar]");
      bar.setAttribute("aria-valuenow", percent);
      bar.firstElementChild.style.transform = `translateX(-${100 - percent}%)`;
      // The rate since the first bytes, so the request's latency isn't in it.
      const now = performance.now();
      first ??= { at: now, bytes: current };
      const ms = now - first.at;
      const bytes = current - first.bytes;
      if (ms > 300 && bytes > 0) {
        const seconds = Math.max(1, Math.ceil(((total - current) * ms) / bytes / 1000));
        document.getElementById("boot-eta").textContent = `Ca ${seconds} s kvar`;
      }
    },
    onSuccess() {
      document.getElementById("boot").remove();
    },
    onFailure() {
      document.getElementById("boot-progress").remove();
      document.getElementById("boot-failed").hidden = false;
    },
  };
}
