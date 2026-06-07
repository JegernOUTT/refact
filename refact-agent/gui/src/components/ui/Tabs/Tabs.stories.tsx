import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { Tabs } from "./Tabs";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Tabs",
  component: Tabs,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Tabs>;

export default meta;
type Story = StoryObj<typeof meta>;

const tabValues = ["overview", "tools", "settings"];

function TabsDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  const [value, setValue] = useState(tabValues[0]);
  const activeIndex = tabValues.indexOf(value);

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Tabs</h3>
          <p className={storyStyles.description}>
            Radix Tabs with scroll-safe list and transform-only sliding indicator.
          </p>
          <Tabs value={value} onValueChange={setValue}>
            <Tabs.List activeIndex={activeIndex} itemCount={tabValues.length}>
              <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
              <Tabs.Trigger value="tools">Tools</Tabs.Trigger>
              <Tabs.Trigger value="settings">Settings</Tabs.Trigger>
            </Tabs.List>
            <Tabs.Content value="overview">
              <div className={storyStyles.previewBox}>Overview content</div>
            </Tabs.Content>
            <Tabs.Content value="tools">
              <div className={storyStyles.previewBox}>Tool settings content</div>
            </Tabs.Content>
            <Tabs.Content value="settings">
              <div className={storyStyles.previewBox}>Settings content</div>
            </Tabs.Content>
          </Tabs>
        </section>
        <section className={`${storyStyles.panel} ${storyStyles.narrowPanel}`} data-appearance="light">
          <p className={storyStyles.description}>Light + narrow container.</p>
          <Tabs defaultValue="one">
            <Tabs.List activeIndex={0} itemCount={4}>
              <Tabs.Trigger value="one">One</Tabs.Trigger>
              <Tabs.Trigger value="two">Two</Tabs.Trigger>
              <Tabs.Trigger value="three">Three</Tabs.Trigger>
              <Tabs.Trigger value="four">Four</Tabs.Trigger>
            </Tabs.List>
          </Tabs>
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  render: () => <TabsDemo />,
};

export const ReducedMotion: Story = {
  render: () => <TabsDemo reducedMotion />,
};
