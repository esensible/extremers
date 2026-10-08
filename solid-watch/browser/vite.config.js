// Dev server for the harness: `npm run dev` (serves the package root; open
// /browser/). Bundles: every fixtures/mutations/<name>/ (bundle.js +
// script.json), plus each directory in SOLID_WATCH_BUNDLES (":"-separated,
// same layout; the directory name is the bundle name), served under /__ext/.
// SOLID_WATCH_FRAGMENTS (":"-separated JSON files) adds app schema fragments.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const list = (v) => (v || "").split(":").filter(Boolean).map((p) => resolve(p));
const extDirs = list(process.env.SOLID_WATCH_BUNDLES);
const fragmentFiles = list(process.env.SOLID_WATCH_FRAGMENTS);

function catalog() {
  const out = [];
  const fx = join(ROOT, "fixtures/mutations");
  for (const name of readdirSync(fx)) {
    // bundles are served raw (/__raw/...), not through Vite's JS transform
    if (existsSync(join(fx, name, "script.json"))) out.push({ name, bundle: `/__raw/fixtures/mutations/${name}/bundle.js`, script: `/__raw/fixtures/mutations/${name}/script.json` });
  }
  extDirs.forEach((d, i) => out.push({ name: basename(d), bundle: `/__ext/${i}/bundle.js`, script: `/__ext/${i}/script.json` }));
  return out;
}

const harness = {
  name: "solid-watch-harness",
  configureServer(server) {
    server.middlewares.use((req, res, next) => {
      const url = req.url.split("?")[0];
      const send = (body, type = "application/json") => {
        res.setHeader("Content-Type", type);
        res.setHeader("Cache-Control", "no-store");
        res.end(body);
      };
      if (url === "/__catalog") return send(JSON.stringify(catalog()));
      if (url === "/__fragments") return send(JSON.stringify(fragmentFiles.map((f) => JSON.parse(readFileSync(f, "utf8")))));
      const raw = url.match(/^\/__raw\/(fixtures\/mutations\/[\w.-]+\/(bundle\.js|script\.json))$/);
      if (raw) return send(readFileSync(join(ROOT, raw[1])), raw[2].endsWith(".js") ? "text/javascript" : "application/json");
      const m = url.match(/^\/__ext\/(\d+)\/(bundle\.js|script\.json)$/);
      if (m && extDirs[+m[1]]) return send(readFileSync(join(extDirs[+m[1]], m[2])), m[2].endsWith(".js") ? "text/javascript" : "application/json");
      next();
    });
  },
};

export default {
  root: ROOT,
  plugins: [harness],
  // yoga-layout's entry uses top-level await
  optimizeDeps: { esbuildOptions: { target: "esnext" } },
  build: { target: "esnext" },
  server: { port: Number(process.env.PORT || 5180), strictPort: true, host: "0.0.0.0", fs: { allow: [ROOT] } },
};
