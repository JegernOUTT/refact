import React from "react";
import { describe, expect, it } from "vitest";

import { render } from "../../../utils/test-utils";
import { Dialog } from "../Dialog";
import styles from "./Select.module.css";
import { Select } from "./Select";

function OpenSelect({
  contentRef,
}: {
  contentRef: React.RefObject<HTMLDivElement>;
}) {
  return (
    <Select open value="anthropic">
      <Select.Trigger aria-label="Base provider" />
      <Select.Content ref={contentRef}>
        <Select.Item value="anthropic">Anthropic</Select.Item>
      </Select.Content>
    </Select>
  );
}

describe("Select", () => {
  it("uses the modal popover layer inside a dialog", () => {
    const contentRef = React.createRef<HTMLDivElement>();

    render(
      <Dialog open>
        <Dialog.Content>
          <Dialog.Title>Add provider instance</Dialog.Title>
          <Dialog.Description>Select a base provider.</Dialog.Description>
          <OpenSelect contentRef={contentRef} />
        </Dialog.Content>
      </Dialog>,
    );

    expect(contentRef.current).toHaveClass(styles.contentInModal);
  });

  it("keeps standalone selects on the regular popover layer", () => {
    const contentRef = React.createRef<HTMLDivElement>();

    render(<OpenSelect contentRef={contentRef} />);

    expect(contentRef.current).not.toHaveClass(styles.contentInModal);
  });
});
