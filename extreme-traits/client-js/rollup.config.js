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
    babel({
      babelHelpers: 'bundled',
      presets: [
        'babel-preset-solid',
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
    html(),
    terser(),
  ]
};
