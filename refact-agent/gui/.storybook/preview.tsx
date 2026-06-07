import type { Decorator, Preview } from "@storybook/react";
import { Theme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "../src/styles/tokens.css";
import "../src/styles/motion.css";
import "../src/styles/responsive.css";
import "../src/lib/render/web.css";
import "./preview.css";

import { initialize, mswLoader } from "msw-storybook-addon";

initialize({
  onUnhandledRequest: (request, print) => {
    if (request.url.startsWith("http://localhost:6006/src/")) {
      return;
    }
    print.warning();
  },
});

type Appearance = "light" | "dark";
type CanvasWidth = "narrow" | "wide";
type ReducedMotion = "on" | "off";

const withDesignSystemModes: Decorator = (Story, context) => {
  const appearance = context.globals.appearance as Appearance;
  const width = context.globals.width as CanvasWidth;
  const reducedMotion = context.globals.reducedMotion as ReducedMotion;

  return (
    <Theme appearance={appearance} accentColor="indigo" grayColor="slate">
      <div
        className={`storybookDesignSystemRoot ${appearance} ${
          reducedMotion === "on" ? "rf-force-reduced" : ""
        }`}
        data-appearance={appearance}
        data-reduced-motion={reducedMotion}
      >
        <div className="storybookDesignSystemCanvas" data-width={width}>
          <Story />
        </div>
      </div>
    </Theme>
  );
};

const preview: Preview = {
  globalTypes: {
    appearance: {
      name: "Appearance",
      description: "Preview light or dark design tokens.",
      defaultValue: "dark",
      toolbar: {
        icon: "circlehollow",
        items: ["light", "dark"],
        dynamicTitle: true,
      },
    },
    width: {
      name: "Width",
      description: "Preview narrow or wide container sizing.",
      defaultValue: "wide",
      toolbar: {
        icon: "browser",
        items: ["narrow", "wide"],
        dynamicTitle: true,
      },
    },
    reducedMotion: {
      name: "Reduced motion",
      description:
        "Visual aid only; production reduced-motion still follows the browser media query.",
      defaultValue: "off",
      toolbar: {
        icon: "time",
        items: ["off", "on"],
        dynamicTitle: true,
      },
    },
  },
  decorators: [withDesignSystemModes],
  parameters: {
    actions: { argTypesRegex: "^on[A-Z].*" },
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    layout: "fullscreen",
  },
  loaders: [mswLoader],
};

export default preview;
