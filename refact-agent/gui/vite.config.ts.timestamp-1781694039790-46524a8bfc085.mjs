// vite.config.ts
import path from "path";
import { defineConfig } from "file:///home/svakhreev/projects/smc/refact/refact-agent/gui/node_modules/vite/dist/node/index.js";
import react from "file:///home/svakhreev/projects/smc/refact/refact-agent/gui/node_modules/@vitejs/plugin-react-swc/index.mjs";
import eslint from "file:///home/svakhreev/projects/smc/refact/refact-agent/gui/node_modules/vite-plugin-eslint/dist/index.mjs";
import { configDefaults, coverageConfigDefaults } from "file:///home/svakhreev/projects/smc/refact/refact-agent/gui/node_modules/vitest/dist/config.js";
import dts from "file:///home/svakhreev/projects/smc/refact/refact-agent/gui/node_modules/vite-plugin-dts/dist/index.mjs";
import { execSync } from "child_process";
var __vite_injected_original_dirname = "/home/svakhreev/projects/smc/refact/refact-agent/gui";
function resolveCommitHash() {
  const envSha = process.env.GITHUB_SHA ?? process.env.CI_COMMIT_SHA ?? process.env.VERCEL_GIT_COMMIT_SHA ?? process.env.BUILD_VCS_NUMBER;
  if (envSha && envSha.length >= 7) return envSha.slice(0, 7);
  try {
    return execSync("git rev-parse --short HEAD", {
      stdio: ["ignore", "pipe", "ignore"]
    }).toString().trim();
  } catch {
    return "unknown";
  }
}
var commitHash = resolveCommitHash();
function makeConfig(library) {
  return defineConfig(({ command, mode }) => {
    const OUT_DIR = library === "browser" ? "dist/chat" : "dist/events";
    const CONFIG = {
      // Build the webpage
      define: {
        "process.env.NODE_ENV": JSON.stringify(mode),
        __REFACT_CHAT_VERSION__: JSON.stringify({
          semver: process.env.npm_package_version,
          commit: commitHash
        }),
        "process.env.DEBUG": JSON.stringify(process.env.DEBUG),
        __REFACT_LSP_PORT__: process.env.REFACT_LSP_PORT
      },
      mode,
      build: {
        emptyOutDir: true,
        outDir: OUT_DIR,
        copyPublicDir: false,
        sourcemap: library === "browser",
        rollupOptions: {
          // TODO: remove when this issue is closed https://github.com/vitejs/vite/issues/15012
          onwarn(warning, defaultHandler) {
            if (warning.code === "SOURCEMAP_ERROR") {
              return;
            }
            defaultHandler(warning);
          }
        }
      },
      plugins: [react()],
      server: {
        proxy: {
          "/v1": process.env.REFACT_LSP_URL ?? "http://127.0.0.1:8001"
        }
      },
      test: {
        retry: 2,
        environment: "happy-dom",
        exclude: [...configDefaults.exclude, "tests/e2e/**", "**/*.spec.ts"],
        coverage: {
          exclude: coverageConfigDefaults.exclude.concat(
            "**/*.stories.@(js|jsx|mjs|ts|tsx)"
          )
        },
        setupFiles: ["./src/utils/test-setup.ts"],
        pool: "forks",
        poolOptions: {
          forks: {
            execArgv: ["--max-old-space-size=4096"],
            maxForks: 4,
            minForks: 1
          }
        }
      },
      css: {
        modules: {}
      }
    };
    if (command !== "serve") {
      CONFIG.mode = "production";
      CONFIG.define = {
        ...CONFIG.define,
        "process.env.NODE_ENV": "'production'"
      };
      CONFIG.plugins?.push([
        // eslint-disable-next-line @typescript-eslint/no-unsafe-call
        eslint({
          exclude: [
            "**/node_modules/**",
            "**/virtual:/**",
            "**/src/features/Buddy/**"
          ]
        })
      ]);
      CONFIG.plugins?.push([
        dts({
          outDir: OUT_DIR,
          rollupTypes: true,
          insertTypesEntry: true
        })
      ]);
      CONFIG.build = {
        ...CONFIG.build,
        lib: {
          entry: library === "browser" ? path.resolve(__vite_injected_original_dirname, "src/lib/index.ts") : path.resolve(__vite_injected_original_dirname, "src/events/index.ts"),
          name: "RefactChat",
          fileName: "index"
        }
      };
    }
    return CONFIG;
  });
}
var vite_config_default = makeConfig("browser");
var nodeConfig = makeConfig("node");
export {
  vite_config_default as default,
  nodeConfig
};
//# sourceMappingURL=data:application/json;base64,ewogICJ2ZXJzaW9uIjogMywKICAic291cmNlcyI6IFsidml0ZS5jb25maWcudHMiXSwKICAic291cmNlc0NvbnRlbnQiOiBbImNvbnN0IF9fdml0ZV9pbmplY3RlZF9vcmlnaW5hbF9kaXJuYW1lID0gXCIvaG9tZS9zdmFraHJlZXYvcHJvamVjdHMvc21jL3JlZmFjdC9yZWZhY3QtYWdlbnQvZ3VpXCI7Y29uc3QgX192aXRlX2luamVjdGVkX29yaWdpbmFsX2ZpbGVuYW1lID0gXCIvaG9tZS9zdmFraHJlZXYvcHJvamVjdHMvc21jL3JlZmFjdC9yZWZhY3QtYWdlbnQvZ3VpL3ZpdGUuY29uZmlnLnRzXCI7Y29uc3QgX192aXRlX2luamVjdGVkX29yaWdpbmFsX2ltcG9ydF9tZXRhX3VybCA9IFwiZmlsZTovLy9ob21lL3N2YWtocmVldi9wcm9qZWN0cy9zbWMvcmVmYWN0L3JlZmFjdC1hZ2VudC9ndWkvdml0ZS5jb25maWcudHNcIjsvLy8gPHJlZmVyZW5jZSB0eXBlcz1cInZpdGVzdFwiIC8+XG5pbXBvcnQgcGF0aCBmcm9tIFwicGF0aFwiO1xuaW1wb3J0IHsgUGx1Z2luT3B0aW9uLCBVc2VyQ29uZmlnLCBkZWZpbmVDb25maWcgfSBmcm9tIFwidml0ZVwiO1xuaW1wb3J0IHJlYWN0IGZyb20gXCJAdml0ZWpzL3BsdWdpbi1yZWFjdC1zd2NcIjtcbmltcG9ydCBlc2xpbnQgZnJvbSBcInZpdGUtcGx1Z2luLWVzbGludFwiO1xuXG5pbXBvcnQgeyBjb25maWdEZWZhdWx0cywgY292ZXJhZ2VDb25maWdEZWZhdWx0cyB9IGZyb20gXCJ2aXRlc3QvY29uZmlnXCI7XG5pbXBvcnQgZHRzIGZyb20gXCJ2aXRlLXBsdWdpbi1kdHNcIjtcblxuaW1wb3J0IHsgZXhlY1N5bmMgfSBmcm9tIFwiY2hpbGRfcHJvY2Vzc1wiO1xuXG5mdW5jdGlvbiByZXNvbHZlQ29tbWl0SGFzaCgpOiBzdHJpbmcge1xuICBjb25zdCBlbnZTaGEgPVxuICAgIHByb2Nlc3MuZW52LkdJVEhVQl9TSEEgPz9cbiAgICBwcm9jZXNzLmVudi5DSV9DT01NSVRfU0hBID8/XG4gICAgcHJvY2Vzcy5lbnYuVkVSQ0VMX0dJVF9DT01NSVRfU0hBID8/XG4gICAgcHJvY2Vzcy5lbnYuQlVJTERfVkNTX05VTUJFUjtcblxuICBpZiAoZW52U2hhICYmIGVudlNoYS5sZW5ndGggPj0gNykgcmV0dXJuIGVudlNoYS5zbGljZSgwLCA3KTtcblxuICB0cnkge1xuICAgIHJldHVybiBleGVjU3luYyhcImdpdCByZXYtcGFyc2UgLS1zaG9ydCBIRUFEXCIsIHtcbiAgICAgIHN0ZGlvOiBbXCJpZ25vcmVcIiwgXCJwaXBlXCIsIFwiaWdub3JlXCJdLFxuICAgIH0pXG4gICAgICAudG9TdHJpbmcoKVxuICAgICAgLnRyaW0oKTtcbiAgfSBjYXRjaCB7XG4gICAgcmV0dXJuIFwidW5rbm93blwiO1xuICB9XG59XG5cbmNvbnN0IGNvbW1pdEhhc2ggPSByZXNvbHZlQ29tbWl0SGFzaCgpO1xuXG4vLyBUT0RPOiByZW1vdmUgZXh0cmEgY29tcGlsZSBzdGVwIHdoZW4gdnNjb2RlIGNhbiBydW4gZXNtb2R1bGVzICBodHRwczovL2dpdGh1Yi5jb20vbWljcm9zb2Z0L3ZzY29kZS9pc3N1ZXMvMTMwMzY3XG5cbi8vIGh0dHBzOi8vdml0ZWpzLmRldi9jb25maWcvXG4vKiogQHR5cGUge2ltcG9ydCgndml0ZScpLlVzZXJDb25maWd9ICovXG5mdW5jdGlvbiBtYWtlQ29uZmlnKGxpYnJhcnk6IFwiYnJvd3NlclwiIHwgXCJub2RlXCIpIHtcbiAgcmV0dXJuIGRlZmluZUNvbmZpZygoeyBjb21tYW5kLCBtb2RlIH0pID0+IHtcbiAgICBjb25zdCBPVVRfRElSID0gbGlicmFyeSA9PT0gXCJicm93c2VyXCIgPyBcImRpc3QvY2hhdFwiIDogXCJkaXN0L2V2ZW50c1wiO1xuICAgIGNvbnN0IENPTkZJRzogVXNlckNvbmZpZyA9IHtcbiAgICAgIC8vIEJ1aWxkIHRoZSB3ZWJwYWdlXG4gICAgICBkZWZpbmU6IHtcbiAgICAgICAgXCJwcm9jZXNzLmVudi5OT0RFX0VOVlwiOiBKU09OLnN0cmluZ2lmeShtb2RlKSxcbiAgICAgICAgX19SRUZBQ1RfQ0hBVF9WRVJTSU9OX186IEpTT04uc3RyaW5naWZ5KHtcbiAgICAgICAgICBzZW12ZXI6IHByb2Nlc3MuZW52Lm5wbV9wYWNrYWdlX3ZlcnNpb24sXG4gICAgICAgICAgY29tbWl0OiBjb21taXRIYXNoLFxuICAgICAgICB9KSxcbiAgICAgICAgXCJwcm9jZXNzLmVudi5ERUJVR1wiOiBKU09OLnN0cmluZ2lmeShwcm9jZXNzLmVudi5ERUJVRyksXG4gICAgICAgIF9fUkVGQUNUX0xTUF9QT1JUX186IHByb2Nlc3MuZW52LlJFRkFDVF9MU1BfUE9SVCxcbiAgICAgIH0sXG4gICAgICBtb2RlLFxuICAgICAgYnVpbGQ6IHtcbiAgICAgICAgZW1wdHlPdXREaXI6IHRydWUsXG4gICAgICAgIG91dERpcjogT1VUX0RJUixcbiAgICAgICAgY29weVB1YmxpY0RpcjogZmFsc2UsXG4gICAgICAgIHNvdXJjZW1hcDogbGlicmFyeSA9PT0gXCJicm93c2VyXCIsXG4gICAgICAgIHJvbGx1cE9wdGlvbnM6IHtcbiAgICAgICAgICAvLyBUT0RPOiByZW1vdmUgd2hlbiB0aGlzIGlzc3VlIGlzIGNsb3NlZCBodHRwczovL2dpdGh1Yi5jb20vdml0ZWpzL3ZpdGUvaXNzdWVzLzE1MDEyXG4gICAgICAgICAgb253YXJuKHdhcm5pbmcsIGRlZmF1bHRIYW5kbGVyKSB7XG4gICAgICAgICAgICBpZiAod2FybmluZy5jb2RlID09PSBcIlNPVVJDRU1BUF9FUlJPUlwiKSB7XG4gICAgICAgICAgICAgIHJldHVybjtcbiAgICAgICAgICAgIH1cblxuICAgICAgICAgICAgZGVmYXVsdEhhbmRsZXIod2FybmluZyk7XG4gICAgICAgICAgfSxcbiAgICAgICAgfSxcbiAgICAgIH0sXG4gICAgICBwbHVnaW5zOiBbcmVhY3QoKV0sXG4gICAgICBzZXJ2ZXI6IHtcbiAgICAgICAgcHJveHk6IHtcbiAgICAgICAgICBcIi92MVwiOiBwcm9jZXNzLmVudi5SRUZBQ1RfTFNQX1VSTCA/PyBcImh0dHA6Ly8xMjcuMC4wLjE6ODAwMVwiLFxuICAgICAgICB9LFxuICAgICAgfSxcbiAgICAgIHRlc3Q6IHtcbiAgICAgICAgcmV0cnk6IDIsXG4gICAgICAgIGVudmlyb25tZW50OiBcImhhcHB5LWRvbVwiLFxuICAgICAgICBleGNsdWRlOiBbLi4uY29uZmlnRGVmYXVsdHMuZXhjbHVkZSwgXCJ0ZXN0cy9lMmUvKipcIiwgXCIqKi8qLnNwZWMudHNcIl0sXG4gICAgICAgIGNvdmVyYWdlOiB7XG4gICAgICAgICAgZXhjbHVkZTogY292ZXJhZ2VDb25maWdEZWZhdWx0cy5leGNsdWRlLmNvbmNhdChcbiAgICAgICAgICAgIFwiKiovKi5zdG9yaWVzLkAoanN8anN4fG1qc3x0c3x0c3gpXCIsXG4gICAgICAgICAgKSxcbiAgICAgICAgfSxcbiAgICAgICAgc2V0dXBGaWxlczogW1wiLi9zcmMvdXRpbHMvdGVzdC1zZXR1cC50c1wiXSxcbiAgICAgICAgcG9vbDogXCJmb3Jrc1wiLFxuICAgICAgICBwb29sT3B0aW9uczoge1xuICAgICAgICAgIGZvcmtzOiB7XG4gICAgICAgICAgICBleGVjQXJndjogW1wiLS1tYXgtb2xkLXNwYWNlLXNpemU9NDA5NlwiXSxcbiAgICAgICAgICAgIG1heEZvcmtzOiA0LFxuICAgICAgICAgICAgbWluRm9ya3M6IDEsXG4gICAgICAgICAgfSxcbiAgICAgICAgfSxcbiAgICAgIH0sXG4gICAgICBjc3M6IHtcbiAgICAgICAgbW9kdWxlczoge30sXG4gICAgICB9LFxuICAgIH07XG5cbiAgICBpZiAoY29tbWFuZCAhPT0gXCJzZXJ2ZVwiKSB7XG4gICAgICBDT05GSUcubW9kZSA9IFwicHJvZHVjdGlvblwiO1xuICAgICAgQ09ORklHLmRlZmluZSA9IHtcbiAgICAgICAgLi4uQ09ORklHLmRlZmluZSxcbiAgICAgICAgXCJwcm9jZXNzLmVudi5OT0RFX0VOVlwiOiBcIidwcm9kdWN0aW9uJ1wiLFxuICAgICAgfTtcblxuICAgICAgQ09ORklHLnBsdWdpbnM/LnB1c2goW1xuICAgICAgICAvLyBlc2xpbnQtZGlzYWJsZS1uZXh0LWxpbmUgQHR5cGVzY3JpcHQtZXNsaW50L25vLXVuc2FmZS1jYWxsXG4gICAgICAgIGVzbGludCh7XG4gICAgICAgICAgZXhjbHVkZTogW1xuICAgICAgICAgICAgXCIqKi9ub2RlX21vZHVsZXMvKipcIixcbiAgICAgICAgICAgIFwiKiovdmlydHVhbDovKipcIixcbiAgICAgICAgICAgIFwiKiovc3JjL2ZlYXR1cmVzL0J1ZGR5LyoqXCIsXG4gICAgICAgICAgXSxcbiAgICAgICAgfSkgYXMgUGx1Z2luT3B0aW9uLFxuICAgICAgXSk7XG5cbiAgICAgIENPTkZJRy5wbHVnaW5zPy5wdXNoKFtcbiAgICAgICAgZHRzKHtcbiAgICAgICAgICBvdXREaXI6IE9VVF9ESVIsXG4gICAgICAgICAgcm9sbHVwVHlwZXM6IHRydWUsXG4gICAgICAgICAgaW5zZXJ0VHlwZXNFbnRyeTogdHJ1ZSxcbiAgICAgICAgfSksXG4gICAgICBdKTtcblxuICAgICAgQ09ORklHLmJ1aWxkID0ge1xuICAgICAgICAuLi5DT05GSUcuYnVpbGQsXG4gICAgICAgIGxpYjoge1xuICAgICAgICAgIGVudHJ5OlxuICAgICAgICAgICAgbGlicmFyeSA9PT0gXCJicm93c2VyXCJcbiAgICAgICAgICAgICAgPyBwYXRoLnJlc29sdmUoX19kaXJuYW1lLCBcInNyYy9saWIvaW5kZXgudHNcIilcbiAgICAgICAgICAgICAgOiBwYXRoLnJlc29sdmUoX19kaXJuYW1lLCBcInNyYy9ldmVudHMvaW5kZXgudHNcIiksXG4gICAgICAgICAgbmFtZTogXCJSZWZhY3RDaGF0XCIsXG4gICAgICAgICAgZmlsZU5hbWU6IFwiaW5kZXhcIixcbiAgICAgICAgfSxcbiAgICAgIH07XG4gICAgfVxuXG4gICAgcmV0dXJuIENPTkZJRztcbiAgfSk7XG59XG5cbmV4cG9ydCBkZWZhdWx0IG1ha2VDb25maWcoXCJicm93c2VyXCIpO1xuXG5leHBvcnQgY29uc3Qgbm9kZUNvbmZpZyA9IG1ha2VDb25maWcoXCJub2RlXCIpO1xuIl0sCiAgIm1hcHBpbmdzIjogIjtBQUNBLE9BQU8sVUFBVTtBQUNqQixTQUFtQyxvQkFBb0I7QUFDdkQsT0FBTyxXQUFXO0FBQ2xCLE9BQU8sWUFBWTtBQUVuQixTQUFTLGdCQUFnQiw4QkFBOEI7QUFDdkQsT0FBTyxTQUFTO0FBRWhCLFNBQVMsZ0JBQWdCO0FBVHpCLElBQU0sbUNBQW1DO0FBV3pDLFNBQVMsb0JBQTRCO0FBQ25DLFFBQU0sU0FDSixRQUFRLElBQUksY0FDWixRQUFRLElBQUksaUJBQ1osUUFBUSxJQUFJLHlCQUNaLFFBQVEsSUFBSTtBQUVkLE1BQUksVUFBVSxPQUFPLFVBQVUsRUFBRyxRQUFPLE9BQU8sTUFBTSxHQUFHLENBQUM7QUFFMUQsTUFBSTtBQUNGLFdBQU8sU0FBUyw4QkFBOEI7QUFBQSxNQUM1QyxPQUFPLENBQUMsVUFBVSxRQUFRLFFBQVE7QUFBQSxJQUNwQyxDQUFDLEVBQ0UsU0FBUyxFQUNULEtBQUs7QUFBQSxFQUNWLFFBQVE7QUFDTixXQUFPO0FBQUEsRUFDVDtBQUNGO0FBRUEsSUFBTSxhQUFhLGtCQUFrQjtBQU1yQyxTQUFTLFdBQVcsU0FBNkI7QUFDL0MsU0FBTyxhQUFhLENBQUMsRUFBRSxTQUFTLEtBQUssTUFBTTtBQUN6QyxVQUFNLFVBQVUsWUFBWSxZQUFZLGNBQWM7QUFDdEQsVUFBTSxTQUFxQjtBQUFBO0FBQUEsTUFFekIsUUFBUTtBQUFBLFFBQ04sd0JBQXdCLEtBQUssVUFBVSxJQUFJO0FBQUEsUUFDM0MseUJBQXlCLEtBQUssVUFBVTtBQUFBLFVBQ3RDLFFBQVEsUUFBUSxJQUFJO0FBQUEsVUFDcEIsUUFBUTtBQUFBLFFBQ1YsQ0FBQztBQUFBLFFBQ0QscUJBQXFCLEtBQUssVUFBVSxRQUFRLElBQUksS0FBSztBQUFBLFFBQ3JELHFCQUFxQixRQUFRLElBQUk7QUFBQSxNQUNuQztBQUFBLE1BQ0E7QUFBQSxNQUNBLE9BQU87QUFBQSxRQUNMLGFBQWE7QUFBQSxRQUNiLFFBQVE7QUFBQSxRQUNSLGVBQWU7QUFBQSxRQUNmLFdBQVcsWUFBWTtBQUFBLFFBQ3ZCLGVBQWU7QUFBQTtBQUFBLFVBRWIsT0FBTyxTQUFTLGdCQUFnQjtBQUM5QixnQkFBSSxRQUFRLFNBQVMsbUJBQW1CO0FBQ3RDO0FBQUEsWUFDRjtBQUVBLDJCQUFlLE9BQU87QUFBQSxVQUN4QjtBQUFBLFFBQ0Y7QUFBQSxNQUNGO0FBQUEsTUFDQSxTQUFTLENBQUMsTUFBTSxDQUFDO0FBQUEsTUFDakIsUUFBUTtBQUFBLFFBQ04sT0FBTztBQUFBLFVBQ0wsT0FBTyxRQUFRLElBQUksa0JBQWtCO0FBQUEsUUFDdkM7QUFBQSxNQUNGO0FBQUEsTUFDQSxNQUFNO0FBQUEsUUFDSixPQUFPO0FBQUEsUUFDUCxhQUFhO0FBQUEsUUFDYixTQUFTLENBQUMsR0FBRyxlQUFlLFNBQVMsZ0JBQWdCLGNBQWM7QUFBQSxRQUNuRSxVQUFVO0FBQUEsVUFDUixTQUFTLHVCQUF1QixRQUFRO0FBQUEsWUFDdEM7QUFBQSxVQUNGO0FBQUEsUUFDRjtBQUFBLFFBQ0EsWUFBWSxDQUFDLDJCQUEyQjtBQUFBLFFBQ3hDLE1BQU07QUFBQSxRQUNOLGFBQWE7QUFBQSxVQUNYLE9BQU87QUFBQSxZQUNMLFVBQVUsQ0FBQywyQkFBMkI7QUFBQSxZQUN0QyxVQUFVO0FBQUEsWUFDVixVQUFVO0FBQUEsVUFDWjtBQUFBLFFBQ0Y7QUFBQSxNQUNGO0FBQUEsTUFDQSxLQUFLO0FBQUEsUUFDSCxTQUFTLENBQUM7QUFBQSxNQUNaO0FBQUEsSUFDRjtBQUVBLFFBQUksWUFBWSxTQUFTO0FBQ3ZCLGFBQU8sT0FBTztBQUNkLGFBQU8sU0FBUztBQUFBLFFBQ2QsR0FBRyxPQUFPO0FBQUEsUUFDVix3QkFBd0I7QUFBQSxNQUMxQjtBQUVBLGFBQU8sU0FBUyxLQUFLO0FBQUE7QUFBQSxRQUVuQixPQUFPO0FBQUEsVUFDTCxTQUFTO0FBQUEsWUFDUDtBQUFBLFlBQ0E7QUFBQSxZQUNBO0FBQUEsVUFDRjtBQUFBLFFBQ0YsQ0FBQztBQUFBLE1BQ0gsQ0FBQztBQUVELGFBQU8sU0FBUyxLQUFLO0FBQUEsUUFDbkIsSUFBSTtBQUFBLFVBQ0YsUUFBUTtBQUFBLFVBQ1IsYUFBYTtBQUFBLFVBQ2Isa0JBQWtCO0FBQUEsUUFDcEIsQ0FBQztBQUFBLE1BQ0gsQ0FBQztBQUVELGFBQU8sUUFBUTtBQUFBLFFBQ2IsR0FBRyxPQUFPO0FBQUEsUUFDVixLQUFLO0FBQUEsVUFDSCxPQUNFLFlBQVksWUFDUixLQUFLLFFBQVEsa0NBQVcsa0JBQWtCLElBQzFDLEtBQUssUUFBUSxrQ0FBVyxxQkFBcUI7QUFBQSxVQUNuRCxNQUFNO0FBQUEsVUFDTixVQUFVO0FBQUEsUUFDWjtBQUFBLE1BQ0Y7QUFBQSxJQUNGO0FBRUEsV0FBTztBQUFBLEVBQ1QsQ0FBQztBQUNIO0FBRUEsSUFBTyxzQkFBUSxXQUFXLFNBQVM7QUFFNUIsSUFBTSxhQUFhLFdBQVcsTUFBTTsiLAogICJuYW1lcyI6IFtdCn0K
