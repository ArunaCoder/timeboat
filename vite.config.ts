import { defineConfig } from "vite";

// Configuração do Vite para o webview do Tauri.
// A porta é fixa (strictPort) porque o tauri.conf.json aponta para ela no devUrl.
// É 1430, e não a 1420 do template, para não colidir com o app do tudolindo
// quando os dois rodam em desenvolvimento ao mesmo tempo.
// Desestruturado, e não `process.env.TAURI_DEV_HOST`: a variável vem da assinatura de
// índice do `ProcessEnv`, que o `noPropertyAccessFromIndexSignature` do tsconfig não deixa
// ler com ponto, e o colchete com literal cairia no `useLiteralKeys` do Biome.
const { TAURI_DEV_HOST: host } = process.env;

export default defineConfig({
  // Tauri cuida do clear screen; preserva os logs do Rust no terminal.
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host ?? false,
    // Omitido (em vez de `undefined`) quando não há host: exactOptionalPropertyTypes
    // não aceita atribuir undefined a propriedade opcional; ausente, o Vite usa o default.
    ...(host ? { hmr: { protocol: "ws", host, port: 1431 } } : {}),
    // Não observar mudanças em src-tauri (recompilação é responsabilidade do Cargo).
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    // O webview é o WebView2 (Chromium), sempre atualizado no Windows.
    target: "es2022",
  },
});
