import type { Decorator, Preview } from "@storybook/react";
import { useEffect } from "react";
import { Theme } from "@radix-ui/themes";
// Keep in sync with src/lib/index.ts — stories must resolve the same global
// cascade as the app (box-sizing reset, ambient type, glass, scrollbars),
// otherwise Storybook measures a different design system than production.
import "@radix-ui/themes/styles.css";
import "../src/styles/tokens.css";
import "../src/styles/base.css";
import "../src/styles/glass.css";
import "../src/styles/motion.css";
import "../src/styles/responsive.css";
import "../src/styles/scrollbar.css";
import "../src/components/Theme/theme-config.css";
import "../src/components/shared/tokens.css";
import "../src/lib/render/web.css";
import "./preview.css";

import { initialize, mswLoader } from "msw-storybook-addon";

initialize({
  onUnhandledRequest: (request, print) => {
    const url = new URL(request.url);
    const isSameOrigin = url.origin === window.location.origin;
    const isApiPath =
      url.pathname.startsWith("/v1/") || url.pathname.startsWith("/p/");
    if (isSameOrigin && !isApiPath) {
      return;
    }
    print.warning();
  },
});

type Appearance = "light" | "dark";
type CanvasWidth = "narrow" | "wide";
type ReducedMotion = "on" | "off";

function isAppearance(value: unknown): value is Appearance {
  return value === "light" || value === "dark";
}

// Portals escape wrapper classes, so mirror the toggles onto <html>:
// src/styles/motion.css has an html[data-reduced-motion="on"] block (audit
// N-09), and tokens.css resolves [data-appearance] / .light|.dark from any
// ancestor, which is what makes light overlays possible (audit N-04).
function applyDocumentModes(
  appearance: Appearance,
  reducedMotion: ReducedMotion,
) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.dataset.reducedMotion = reducedMotion === "on" ? "on" : "off";
  root.dataset.appearance = appearance;
  root.classList.toggle("light", appearance === "light");
  root.classList.toggle("dark", appearance === "dark");
  root.style.colorScheme = appearance;
}

function DocumentModes({
  appearance,
  reducedMotion,
}: {
  appearance: Appearance;
  reducedMotion: ReducedMotion;
}) {
  useEffect(() => {
    applyDocumentModes(appearance, reducedMotion);
  }, [appearance, reducedMotion]);

  return null;
}

const withDesignSystemModes: Decorator = (Story, context) => {
  // A story can pin its own appearance (parameters.appearance) so portaled
  // overlays - which mount at document.body, outside any story wrapper - are
  // rendered in the requested mode. The toolbar global is the fallback.
  const storyAppearance = (context.parameters as { appearance?: unknown })
    .appearance;
  const appearance: Appearance = isAppearance(storyAppearance)
    ? storyAppearance
    : (context.globals.appearance as Appearance);
  const width = context.globals.width as CanvasWidth;
  // Reduced motion mirrors appearance: a story can pin it via parameters so
  // the html[data-reduced-motion] contract reaches portaled overlays too.
  const storyReducedMotion = (context.parameters as { reducedMotion?: unknown })
    .reducedMotion;
  const reducedMotion: ReducedMotion =
    storyReducedMotion === "on" || storyReducedMotion === "off"
      ? storyReducedMotion
      : (context.globals.reducedMotion as ReducedMotion);

  // Harness stories read documentElement.dataset.appearance during their first
  // render (resolveStoryAppearance falls back to dark): apply modes
  // synchronously before children mount so that read never races the effect.
  applyDocumentModes(appearance, reducedMotion);

  return (
    <Theme appearance={appearance} accentColor="indigo" grayColor="slate">
      <DocumentModes appearance={appearance} reducedMotion={reducedMotion} />
      <div
        className={`storybookDesignSystemRoot ${appearance} ${
          reducedMotion === "on" ? "rf-force-reduced" : ""
        }`}
        data-appearance={appearance}
        data-reduced-motion={reducedMotion}
        style={{ colorScheme: appearance }}
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
