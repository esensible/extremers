// Race watch bundle -> dist/watch.js (minified ES2020 IIFE), built with
// solid-watch's bundle config (two-pass babel, universal JSX, _tmpl$ guard).
import { solidWatchBundle } from "solid-watch/rollup";

export default solidWatchBundle({ input: "src/index.jsx", file: "dist/watch.js" });
