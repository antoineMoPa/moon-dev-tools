// What Ghostty's VT engine imports from its host when it is built to wasm: `env.log`, which it
// hands each log message as a pointer and a length into the module's memory. build.rs points
// the module's `env` import here.
//
// The message is in memory only the module can read, so the page hands this the module's own
// reader once it has started - see web/index.html and `ghostty_log` in src/web.rs.

let readMessage = null;

export function log(ptr, len) {
  if (readMessage === null) {
    throw new Error("Ghostty logged before the page gave it a reader - see web/index.html");
  }
  readMessage(ptr, len);
}

export function readLogsWith(reader) {
  readMessage = reader;
}
