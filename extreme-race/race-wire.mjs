// The race engine's wire descriptors, compiled at build time for the two
// clients (client-js/, the Kindle page, and watch-js/, the watch bundle).
//
// The view the host publishes and the events the clients send are the
// engine's (engine/race/view.schema.json and event.schema.json), encoded
// with the wire codec (wire/js). The descriptors are compiled here with
// wire.compile, in node, so the bundles carry only the codec (wire/js/codec.js)
// and the descriptor bytes, not the compiler; a schema change rebuilds them.
//
//   import { raceWire } from "../race-wire.mjs";   // a rollup plugin
//   import { VIEW, EVENT } from "race-wire";       // in a client's source:
//                                                  // the descriptors, Uint8Arrays
//
// raceDescriptors() gives the same bytes to node scripts and tests.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const wire = require("../wire/js/index.js");

export const SCHEMAS = {
  view: fileURLToPath(new URL("../engine/race/view.schema.json", import.meta.url)),
  event: fileURLToPath(new URL("../engine/race/event.schema.json", import.meta.url)),
};

// The view and event descriptors (Uint8Arrays), compiled from the schemas.
export function raceDescriptors() {
  return {
    view: wire.compile(JSON.parse(readFileSync(SCHEMAS.view, "utf8"))),
    event: wire.compile(JSON.parse(readFileSync(SCHEMAS.event, "utf8"))),
  };
}

const ID = "race-wire";
const RESOLVED = "\0" + ID;

// Rollup: `import { VIEW, EVENT } from "race-wire"` is a module of the two
// descriptors as byte literals.
export function raceWire() {
  return {
    name: "race-wire",
    resolveId(id) {
      return id === ID ? RESOLVED : null;
    },
    load(id) {
      if (id !== RESOLVED) return null;
      this.addWatchFile(SCHEMAS.view);
      this.addWatchFile(SCHEMAS.event);
      const d = raceDescriptors();
      const bytes = (b) => `new Uint8Array([${Array.from(b).join(",")}])`;
      return `export var VIEW = ${bytes(d.view)};\nexport var EVENT = ${bytes(d.event)};\n`;
    },
  };
}
