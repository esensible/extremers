// Builds each test/bundles/<name>/index.jsx to fixtures/mutations/<name>/bundle.js:
// an ES2020 IIFE, unminified, the same pipeline an app bundle uses.
import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { solidWatchBundle } from "../js/rollup.js";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const names = readdirSync(ROOT + "test/bundles");

export default names.map((name) =>
  solidWatchBundle({
    input: `${ROOT}test/bundles/${name}/index.jsx`,
    file: `${ROOT}fixtures/mutations/${name}/bundle.js`,
    minify: false,
    solidWatch: ROOT + "js/index.js",
  }),
);
