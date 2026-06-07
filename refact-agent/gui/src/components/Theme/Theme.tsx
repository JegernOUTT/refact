import React from "react";
import { Theme as RadixTheme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "../../styles/tokens.css";
import "../../styles/motion.css";
import "../../styles/responsive.css";
import "./theme-config.css";
import "../shared/tokens.css";
import { useAppearance, useConfig } from "../../hooks";

export type ThemeProps = {
  children: JSX.Element;
  appearance?: "inherit" | "light" | "dark";

  accentColor?:
    | "tomato"
    | "red"
    | "ruby"
    | "crimson"
    | "pink"
    | "plum"
    | "purple"
    | "violet"
    | "iris"
    | "indigo"
    | "blue"
    | "cyan"
    | "teal"
    | "jade"
    | "green"
    | "grass"
    | "brown"
    | "orange"
    | "sky"
    | "mint"
    | "lime"
    | "yellow"
    | "amber"
    | "gold"
    | "bronze"
    | "gray";

  grayColor?: "gray" | "mauve" | "slate" | "sage" | "olive" | "sand" | "auto";
  panelBackground?: "solid" | "translucent";
  radius?: "none" | "small" | "medium" | "large" | "full";
  scaling?: "90%" | "95%" | "100%" | "105%" | "110%";
  hasBackground?: boolean;
};

export const Theme: React.FC<ThemeProps> = (props) => {
  const { host, themeProps } = useConfig();
  const { appearance } = useAppearance();

  return (
    <RadixTheme
      {...themeProps}
      {...props}
      appearance={appearance}
      data-host={host}
      data-appearance={appearance}
    />
  );
};
