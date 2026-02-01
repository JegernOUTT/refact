import React from "react";
import { Theme as RadixTheme } from "@radix-ui/themes";
import { MoonIcon, SunIcon } from "@radix-ui/react-icons";
import "@radix-ui/themes/styles.css";
import "./theme-config.css";
import styles from "./Theme.module.css";
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

const ThemeWithDarkMode: React.FC<ThemeProps> = ({ children, ...props }) => {
  const { isDarkMode, toggle } = useAppearance();
  const Icon = isDarkMode ? MoonIcon : SunIcon;
  return (
    <RadixTheme {...props} appearance={isDarkMode ? "dark" : "light"}>
      <button
        type="button"
        className={styles.themeToggle}
        title="toggle dark mode"
        onClick={toggle}
        aria-label="Toggle dark mode"
      >
        <Icon />
      </button>
      {/** TODO: remove this in production */}
      {/** use cmd + c to open and close */}
      {/* <ThemePanel defaultOpen={false} /> */}
      {children}
    </RadixTheme>
  );
};

export const Theme: React.FC<ThemeProps> = (props) => {
  const { host, themeProps } = useConfig();
  const { appearance } = useAppearance();

  if (host === "web") {
    return (
      <ThemeWithDarkMode {...themeProps} {...props} appearance={appearance} />
    );
  }

  return <RadixTheme {...themeProps} {...props} appearance={appearance} />;
};
