import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";
import { within, userEvent } from "@storybook/testing-library";

import { Select } from "./Select";
import storyStyles from "../Control.stories.module.css";

const meta = {
  title: "UI/Select",
  component: Select,
  parameters: { layout: "fullscreen" },
} satisfies Meta<typeof Select>;

export default meta;
type Story = StoryObj<typeof meta>;

function SelectDemo({
  defaultOpen = false,
  reducedMotion = false,
}: {
  defaultOpen?: boolean;
  reducedMotion?: boolean;
}) {
  const [value, setValue] = useState("agent");

  return (
    <div className={reducedMotion ? storyStyles.reducedMotion : undefined}>
      <div className={storyStyles.storyShell}>
        <section className={storyStyles.panel}>
          <h3 className={storyStyles.title}>Select</h3>
          <p className={storyStyles.description}>
            Panel-less Radix Select rows with grouped items, selected tint,
            hover state, and clamped overlay surface.
          </p>
          <div className={storyStyles.row}>
            <Select
              defaultOpen={defaultOpen}
              value={value}
              onValueChange={setValue}
            >
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
        {/* The light half must paint its own canvas and flip color-scheme,
            otherwise it inherits dark UA colors and reads as token soup
            (audit N-03). */}
        <section
          className={`${storyStyles.panel} ${storyStyles.narrowPanel} light`}
          data-appearance="light"
        >
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

async function openFirstSelect(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const triggers = [
    ...canvas.queryAllByRole("combobox"),
    ...canvas.queryAllByRole("button"),
  ];
  const trigger = triggers.find((element) => !element.hasAttribute("disabled"));
  if (trigger && trigger.getAttribute("aria-expanded") !== "true") {
    await userEvent.click(trigger);
  }
}

export const States: Story = {
  render: () => <SelectDemo />,
  // Opening the menu is the point of this story: keep the popup coverage here
  // instead of only in the reduced-motion twin (audit N-15).
  play: ({ canvasElement }) => openFirstSelect(canvasElement),
};

export const ReducedMotion: Story = {
  // Pin the html attribute so the portaled overlay stops animating too.
  parameters: { reducedMotion: "on" },
  render: () => <SelectDemo defaultOpen reducedMotion />,
};

// The listbox is portaled to document.body, so a side-by-side light panel can
// never show it in light mode. parameters.appearance pins the whole preview
// (including the portal root) to light instead (audit N-04).
export const LightOverlay: Story = {
  parameters: { appearance: "light" },
  render: () => <SelectDemo />,
  play: ({ canvasElement }) => openFirstSelect(canvasElement),
};
