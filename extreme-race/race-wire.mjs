// The race engine's wire descriptors, compiled at build time for the two
// clients (client-js/, the Kindle page, and watch-js/, the watch bundle):
// the view the host publishes and the events the clients send are the
// engine's (engine/race/view.schema.json and event.schema.json), through
// wire/js/rollup.js.
//
//   import { raceWire } from "../race-wire.mjs";   // a rollup plugin
//   import { VIEW, EVENT } from "race-wire";       // in a client's source:
//                                                  // the descriptors, Uint8Arrays
//
// raceDescriptors() gives the same bytes to node scripts and tests.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { wirePlugin, descriptors } = require("../wire/js/rollup.js");

export const SCHEMAS = {
  view: fileURLToPath(new URL("../engine/race/view.schema.json", import.meta.url)),
  event: fileURLToPath(new URL("../engine/race/event.schema.json", import.meta.url)),
};

const EXPORTS = { VIEW: SCHEMAS.view, EVENT: SCHEMAS.event };

// The view and event descriptors (Uint8Arrays), compiled from the schemas.
export function raceDescriptors() {
  const d = descriptors(EXPORTS);
  return { view: d.VIEW, event: d.EVENT };
}

// Rollup: `import { VIEW, EVENT } from "race-wire"` is a module of the two
// descriptors as byte literals.
export function raceWire() {
  return wirePlugin({ id: "race-wire", schemas: EXPORTS });
}
