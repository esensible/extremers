// import { babel } from '@rollup/plugin-babel';
import babel from '@rollup/plugin-babel';
import html from '@rollup/plugin-html';
import postcss from 'rollup-plugin-postcss';
import terser from '@rollup/plugin-terser';

function uuid(length) {
  return Array.from({ length }, () => Math.random().toString(36)[2]).join('');
}

export default {
  input: 'src/index.jsx',
  base: 'static/',
  output: {
    format: 'umd',
    name: 'main',
    file: 'dist/bundle-' + uuid(8) + '.js',
  },
  plugins: [
    babel({
      exclude: 'node_modules/**',
      presets: [
        [
          "@babel/preset-env",
          {
            targets: {
                browsers: ["last 2 versions", "IE 11"],
            },
          }
        ],
      ],
      plugins: [
        ["@babel/plugin-transform-react-jsx", {
          pragma: "h",
          pragmaFrag: "Not supported",
        }],
      ]
    }),
    postcss({
      extract: true,
    }),
    html(),
    terser()
  ]
};
