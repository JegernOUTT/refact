import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type { StorybookConfig } from "@storybook/react-vite";
import type { PluginOption } from "vite";

const staticDirs = ["../public"];

function withoutDeclarationPlugins(
  plugins: PluginOption[] | undefined,
): PluginOption[] | undefined {
  if (!plugins) {
    return plugins;
  }

  return plugins
    .map((plugin) =>
      Array.isArray(plugin) ? withoutDeclarationPlugins(plugin) : plugin,
    )
    .filter((plugin) => {
      if (Array.isArray(plugin)) {
        return plugin.length > 0;
      }
      if (!plugin || !("name" in plugin)) {
        return true;
      }
      return plugin.name !== "vite:dts";
    }) as PluginOption[];
}

if (existsSync(resolve(__dirname, "../dist"))) {
  staticDirs.push("../dist");
}

const config: StorybookConfig = {
  stories: ["../src/**/*.mdx", "../src/**/*.stories.@(js|jsx|mjs|ts|tsx)"],
  addons: [
    "@storybook/addon-links",
    "@storybook/addon-essentials",
    "@storybook/addon-onboarding",
    "@storybook/addon-interactions",
  ],
  framework: {
    name: "@storybook/react-vite",
    options: {},
  },
  docs: {
    autodocs: "tag",
  },
  viteFinal: (config) => {
    const server = {
      ...config.server,
      proxy: {
        "/v1": process.env.REFACT_LSP_URL ?? "http://127.0.0.1:8001",
      },
    };
    const build = {
      ...config.build,
      lib: undefined,
    };
    const plugins = withoutDeclarationPlugins(config.plugins);

    return { ...config, build, plugins, server };
  },
  staticDirs,
};
export default config;
