// vite.config.js
import { defineConfig } from 'vite';

export default defineConfig({
  esbuild: {
    jsxFactory: 'h',
    jsxFragment: 'Fragment',
    jsxInject: `import { h } from 'silkjs'`,
    include: /\.jsx?$/
  }
});