// import resolve from '@rollup/plugin-node-resolve';
// import terser from '@rollup/plugin-terser';
import getBabelOutputPlugin from '@rollup/plugin-babel';
import html from '@rollup/plugin-html';
import css from 'rollup-plugin-css-only'



let pluginOptions = [
  html( ),
  css(  {output: 'style.css'} ),
  getBabelOutputPlugin({
    exclude: 'node_modules/**',
  }), 
  // terser(),
];

export default {
  input: 'src/index.js',
  output: {
    // name: 'index',   // for external calls (need exports)
    dir: 'dist',
    // file: 'dist/index.min.js',
    format: 'umd',
    assetFileNames: 'assets/[name]-[hash][extname]',
  },
  plugins: pluginOptions,
};