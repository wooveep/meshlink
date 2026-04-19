import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'node:path'

export default defineConfig({
  base: '/admin/',
  plugins: [vue()],
  build: {
    outDir: resolve(__dirname, '../internal/adminui/dist'),
    emptyOutDir: true
  }
})
