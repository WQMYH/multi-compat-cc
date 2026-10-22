import { build } from "vite";
import { copyFile } from "node:fs/promises";

await build({
  configFile: false,
  envFile: false,
  ssr: { noExternal: true },
  build: {
    ssr: "src-runtime/qoder-auth-host.mjs",
    target: "node18",
    outDir: "src-tauri/resources/qoder",
    emptyOutDir: true,
    rollupOptions: { output: { entryFileNames: "qoder-auth-host.mjs" } },
  },
});

await copyFile(
  "node_modules/@qodercn-ai/qodercn-agent-sdk/LICENSE",
  "src-tauri/resources/qoder/QODER-SDK-LICENSE.txt",
);
