// Rollup config for a solid-watch bundle: JSX -> ES2020 IIFE that sets
// globalThis.__app and uses no browser globals.
//
//   import { solidWatchBundle } from "solid-watch/rollup";
//   export default solidWatchBundle({ input: "src/index.jsx", file: "dist/watch.js" });
//
// The build plugins are resolved from solid-watch's own node_modules (run
// `npm install` in solid-watch); solid-js is deduplicated to the app's copy,
// so the renderer and the app share one reactive runtime.
import babel from "@rollup/plugin-babel";
import resolve from "@rollup/plugin-node-resolve";
import presetSolid from "babel-preset-solid";
import presetEnv from "@babel/preset-env";
import terser from "@rollup/plugin-terser";
import { fileURLToPath } from "node:url";

const INDEX = fileURLToPath(new URL("./index.js", import.meta.url));

// babel-preset-solid hoists template declarations (`_tmpl$`); if one is ever
// lost the bundle fails at runtime with "_tmpl$ is not defined". Universal
// output shouldn't have any, so any left over is a build error.
const tmplGuard = {
  name: "solid-watch-tmpl-guard",
  generateBundle(_, bundle) {
    for (const chunk of Object.values(bundle)) {
      if (chunk.code && chunk.code.includes("_tmpl$")) this.error(`${chunk.fileName} still contains _tmpl$`);
    }
  },
};

export function solidWatchBundle({ input, file, minify = true, solidWatch = INDEX, plugins = [] }) {
  const out = [
    // "solid-watch" (babel's moduleName and the app's imports) -> this package
    {
      name: "solid-watch-resolve",
      resolveId(id) {
        if (id === "solid-watch") return solidWatch;
        if (id === "solid-watch/renderer") return fileURLToPath(new URL("./renderer.js", import.meta.url));
        return null;
      },
    },
    // the client (reactive) build of solid-js, not the server one
    resolve({ dedupe: ["solid-js"], browser: true, exportConditions: ["browser"] }),
    // Two passes, as in the client-js configs: Solid's JSX first, then
    // preset-env on the result; in one pass a template declaration was lost.
    babel({
      babelHelpers: "bundled",
      extensions: [".js", ".jsx"],
      babelrc: false,
      configFile: false,
      presets: [[presetSolid, { generate: "universal", moduleName: "solid-watch" }]],
    }),
    babel({
      babelHelpers: "bundled",
      extensions: [".js", ".jsx"],
      babelrc: false,
      configFile: false,
      presets: [[presetEnv, { targets: { esmodules: true }, bugfixes: true }]],
    }),
    ...(minify ? [terser({ ecma: 2020 })] : []),
    ...plugins,
    tmplGuard,
  ];
  return { input, output: { format: "iife", file }, plugins: out };
}
