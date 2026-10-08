import babel from '@rollup/plugin-babel';
import html from '@rollup/plugin-html';
import postcss from 'rollup-plugin-postcss';
import terser from '@rollup/plugin-terser';
import resolve from '@rollup/plugin-node-resolve';
import image from '@rollup/plugin-image';

function uuid(length) {
  return Array.from({ length }, () => Math.random().toString(36)[2]).join('');
}

export default {
  input: 'src/index.jsx',
  output: {
    format: 'umd',
    name: 'main',
    file: 'dist/bundle-' + uuid(8) + '.js',
  },
  cache: false,
  plugins: [
    resolve(),
    image(),
    // Two passes: Solid's JSX first, then preset-env on the result. In one
    // pass, preset-env's transforms can make babel-preset-solid lose a
    // template declaration (seen in the tune client: "ReferenceError: _tmpl$
    // is not defined", a blank page).
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
      title: 'Extreme Race',
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
