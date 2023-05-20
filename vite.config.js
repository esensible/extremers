import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [
    react({
      jsxImportSource: 'silkjs',
    }),
  ],
  server: {
    proxy: {
      '/sync': {
        target: 'http://localhost:8080',
        changeOrigin: true,
        // rewrite: (path) => path.replace(/^\/sync/, '/sync'),
        method: 'GET'
      },
      '/event': {
        target: 'http://localhost:8080',
        changeOrigin: true,
        method: 'POST',
        // onProxyReq: function(proxyReq, req, res) {
        // //   if(req.body) {
        //       let bodyData = JSON.stringify(req.body);
        //       // incase if content-type is application/x-www-form-urlencoded -> we need to change to application/json
        //       proxyReq.setHeader('Content-Type','application/json');
        //       proxyReq.setHeader('Content-Length', Buffer.byteLength(bodyData));
        //       // stream the content
        //       proxyReq.write(bodyData);
        // //   }
        // },   
        configure: (proxy, _options) => {
          proxy.on('error', (err, _req, _res) => {
            console.log('proxy error', err);
          });
          proxy.on('proxyReq', (proxyReq, req, _res) => {
            console.log('Sending Request to the Target:', req.method, req.url, req.headers);
          });
          proxy.on('proxyRes', (proxyRes, req, _res) => {
            console.log('Received Response from the Target:', proxyRes.statusCode, req.url);
          });
        },   
      },
    },
  },  
})
