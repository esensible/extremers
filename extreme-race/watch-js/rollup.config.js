// Race watch bundle -> dist/watch.js (minified ES2020 IIFE), built with
// solid-watch's bundle config (two-pass babel, universal JSX, _tmpl$ guard).
// Ahead of its plugins: the race engine's wire descriptors, compiled at build
// time ("race-wire", ../race-wire.mjs), and commonjs for the wire codec
// (wire/js/codec.js is CommonJS / a classic script, not an ES module).
import commonjs from "@rollup/plugin-commonjs";
import { solidWatchBundle } from "solid-watch/rollup";
import { raceWire } from "../race-wire.mjs";

const bundle = solidWatchBundle({ input: "src/index.jsx", file: "dist/watch.js" });

export default { ...bundle, plugins: [raceWire(), commonjs(), ...bundle.plugins] };
