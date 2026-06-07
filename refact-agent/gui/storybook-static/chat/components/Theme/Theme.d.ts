import React from "react";
export type ThemeProps = {
    children: JSX.Element;
    appearance?: "inherit" | "light" | "dark";
    accentColor?: "tomato" | "red" | "ruby" | "crimson" | "pink" | "plum" | "purple" | "violet" | "iris" | "indigo" | "blue" | "cyan" | "teal" | "jade" | "green" | "grass" | "brown" | "orange" | "sky" | "mint" | "lime" | "yellow" | "amber" | "gold" | "bronze" | "gray";
    grayColor?: "gray" | "mauve" | "slate" | "sage" | "olive" | "sand" | "auto";
    panelBackground?: "solid" | "translucent";
    radius?: "none" | "small" | "medium" | "large" | "full";
    scaling?: "90%" | "95%" | "100%" | "105%" | "110%";
    hasBackground?: boolean;
};
export declare const Theme: React.FC<ThemeProps>;
