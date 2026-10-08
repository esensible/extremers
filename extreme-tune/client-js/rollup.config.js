import babel from '@rollup/plugin-babel';
import html from '@rollup/plugin-html';
import postcss from 'rollup-plugin-postcss';
import terser from '@rollup/plugin-terser';
import resolve from '@rollup/plugin-node-resolve';
import image from '@rollup/plugin-image';

export default {
  input: 'src/index.jsx',
  output: {
    format: 'umd',
    name: 'main',
    // named after its contents, not at random: the package's content hash
    // (tools/package) covers the bundle, so the same sources must make the
    // same bytes; a new bundle still gets a new name (no stale cache)
    dir: 'dist',
    entryFileNames: 'bundle-[hash].js',
  },
  cache: false,
  plugins: [
    resolve(),
    image(),
    // Two passes: Solid's JSX first, then preset-env on the result. In one
    // pass, preset-env's transforms made babel-preset-solid lose a template
    // declaration (the first JSX in a file, here formatDeviation's), and the
    // page threw "ReferenceError: _tmpl$ is not defined" before rendering.
    babel({
      babelHelpers: 'bundled',
      presets: ['babel-preset-solid'],
    }),
    babel({
      babelHelpers: 'bundled',
      presets: [
        [
          "@babel/preset-env",
          {
            targets: {
              browsers: ["last 2 versions", "IE 11"],
            },
          },
        ],
      ],
    }),
    postcss({
      extract: true,
      minimize: true,
    }),
    html({
      // The host serves this page at / while its package is active, and its
      // other files at /pkg/<id>/ (tools/package sets the path): absolute,
      // so they load from the package whatever the page's URL.
      publicPath: process.env.EXTREMERS_PUBLIC_PATH || '',
      title: 'Extreme Tune',
      // The Kindle browser is an old mobile WebKit: it ignores CSS
      // touch-action but honours the viewport meta, so this is what stops
      // pinch/double-tap zoom (water droplets look like a pinch).
      meta: [
        { charset: 'utf-8' },
        {
          name: 'viewport',
          content: 'width=device-width, initial-scale=1, minimum-scale=1, maximum-scale=1, user-scalable=no',
        },
      ],
    }),
    terser(),
  ]
};
