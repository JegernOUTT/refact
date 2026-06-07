import type { Meta, StoryObj } from "@storybook/react";

import { Switch } from "./Switch";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Switch",
  component: Switch,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Switch>;

export default meta;
type Story = StoryObj<typeof meta>;

function SwitchDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Switch</h3>
          <p className={storyStyles.description}>Token styled Radix Switch with transform-only spring thumb.</p>
          <div className={storyStyles.row}>
            <Switch label="Enabled" defaultChecked />
            <Switch label="Off" />
            <Switch label="Disabled" disabled />
          </div>
        </section>
        <section className={`${storyStyles.panel} ${storyStyles.narrowPanel}`} data-appearance="light">
          <p className={storyStyles.description}>Light + narrow container.</p>
          <Switch label="Light theme" defaultChecked />
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  render: () => <SwitchDemo />,
};

export const ReducedMotion: Story = {
  render: () => <SwitchDemo reducedMotion />,
};
