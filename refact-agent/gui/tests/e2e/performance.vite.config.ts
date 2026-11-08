import { defineConfig } from "vite";
import react from "@vitejs/plugin-react-swc";
import { execFileSync } from "node:child_process";

export default defineConfig({
  plugins: [
    react(),
    {
      name: "performance-baseline-source",
      enforce: "pre",
      transform(code, id) {
        if (id.endsWith("/src/features/App.tsx")) {
          return code.replace(
            "const WorkspaceApp: React.FC<AppProps> = ({ style }: AppProps) => {",
            "const WorkspaceApp: React.FC<AppProps> = ({ style }: AppProps) => { Reflect.set(window, '__workspaceAppRenders', (Reflect.get(window, '__workspaceAppRenders') ?? 0) + 1);",
          );
        }
      },
      load(id) {
        if (process.env.UI_PERFORMANCE_BASELINE !== "1") return;
        const relative = id.split("/refact-agent/gui/")[1];
        if (
          relative === "src/features/App.tsx" ||
          relative === "src/features/Chat/Thread/selectors.ts"
        ) {
          return execFileSync(
            "git",
            ["show", `be51a715:refact-agent/gui/${relative}`],
            { encoding: "utf8" },
          );
        }
      },
    },
  ],
  define: {
    __REFACT_LSP_PORT__: "8001",
    __REFACT_CHAT_VERSION__: JSON.stringify({
      semver: "performance-fixture",
      commit: "isolated",
    }),
  },
  build: {
    outDir: "dist/ui-performance",
    rollupOptions: { input: "tests/e2e/route-showcase.html" },
  },
});
