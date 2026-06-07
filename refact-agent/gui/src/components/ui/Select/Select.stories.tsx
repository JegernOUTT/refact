import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";

import { Select } from "./Select";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Select",
  component: Select,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Select>;

export default meta;
type Story = StoryObj<typeof meta>;

function SelectDemo({ reducedMotion = false }: { reducedMotion?: boolean }) {
  const [value, setValue] = useState("agent");

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Select</h3>
          <p className={storyStyles.description}>
            Token styled Radix Select with grouped items, separators, disabled states, and clamped overlay surface.
          </p>
          <div className={storyStyles.row}>
            <Select value={value} onValueChange={setValue}>
              <Select.Trigger placeholder="Choose mode" />
              <Select.Content maxHeight="260px" maxWidth="320px">
                <Select.Group>
                  <Select.Label>Modes</Select.Label>
                  <Select.Item value="agent">Agent</Select.Item>
                  <Select.Item value="explore">Explore</Select.Item>
                  <Select.Item value="planner">Planner</Select.Item>
                </Select.Group>
                <Select.Separator />
                <Select.Item value="disabled" disabled>
                  Disabled mode
                </Select.Item>
              </Select.Content>
            </Select>
            <Select disabled value="locked" onValueChange={() => undefined}>
              <Select.Trigger />
              <Select.Content>
                <Select.Item value="locked">Locked</Select.Item>
              </Select.Content>
            </Select>
          </div>
        </section>
        <section className={`${storyStyles.panel} ${storyStyles.narrowPanel}`} data-appearance="light">
          <p className={storyStyles.description}>Light + narrow container.</p>
          <Select defaultValue="small">
            <Select.Trigger />
            <Select.Content maxWidth="240px">
              <Select.Item value="small">Small</Select.Item>
              <Select.Item value="medium">Medium</Select.Item>
              <Select.Item value="large">Large</Select.Item>
            </Select.Content>
          </Select>
        </section>
      </div>
    </div>
  );
}

export const States: Story = {
  render: () => <SelectDemo />,
};

export const ReducedMotion: Story = {
  render: () => <SelectDemo reducedMotion />,
};
