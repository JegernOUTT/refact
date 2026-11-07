import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AnimatedList } from "./AnimatedList";

describe("AnimatedList", () => {
  it("can render semantic list elements", () => {
    render(
      <AnimatedList as="ul">
        <li>One</li>
        <li>Two</li>
      </AnimatedList>,
    );

    expect(screen.getByRole("list")).toBeInTheDocument();
  });
});
