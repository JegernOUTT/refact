import type { Meta, StoryObj } from "@storybook/react";
import { userEvent, within } from "@storybook/testing-library";
import { DialogImage } from "./DialogImage";
import { FIXTURE_IMAGE_DATA_URI } from "../../__fixtures__/images";

// The image lightbox had zero story coverage (audit L-25); the fixture is a
// real 480x270 PNG so aspect handling and object-fit are genuinely exercised.
const meta: Meta<typeof DialogImage> = {
  title: "Components/DialogImage",
  component: DialogImage,
  parameters: { layout: "centered" },
};

export default meta;
type Story = StoryObj<typeof DialogImage>;

export const Trigger: Story = {
  args: {
    src: FIXTURE_IMAGE_DATA_URI,
    alt: "Fixture screenshot",
    size: "8",
  },
};

export const OpenLightbox: Story = {
  args: {
    src: FIXTURE_IMAGE_DATA_URI,
    alt: "Fixture screenshot",
    size: "8",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const trigger = await canvas.findByRole("button", {
      name: /open image/i,
    });
    await userEvent.click(trigger);
  },
};
