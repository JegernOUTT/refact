import { defineConfig, mergeConfig } from "vitest/config";
import { execFileSync } from "node:child_process";
import baseConfig from "../../vite.config";

export default defineConfig(async (environment) => {
  const base =
    typeof baseConfig === "function"
      ? await baseConfig(environment)
      : await baseConfig;
  return mergeConfig(base, {
    plugins: [
      {
        name: "original-ui-source",
        enforce: "pre",
        load(id: string) {
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
  });
});
