import { Theme } from "@radix-ui/themes";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import cardStyles from "./Card.module.css";
import { Card, GlassCard } from "./Card";
import surfaceStyles from "../Surface/Surface.module.css";

function renderCard(element: React.ReactElement) {
  return render(<Theme>{element}</Theme>);
}

describe("Card", () => {
  it("keeps the default surface variant unchanged", () => {
    renderCard(<Card>Default card</Card>);

    const card = screen.getByText("Default card");

    expect(card).toHaveClass(surfaceStyles.surface1);
    expect(card).toHaveClass(cardStyles.paddingMd);
    expect(card).not.toHaveAttribute("data-selected");
  });

  it("renders the glass surface variant", () => {
    renderCard(<Card variant="glass">Glass card</Card>);

    const card = screen.getByText("Glass card");

    expect(card).toHaveClass(surfaceStyles.glass);
    expect(card).toHaveAttribute("data-card-variant", "glass");
  });

  it("keeps selected glass cards on the glass surface", () => {
    renderCard(
      <Card selected variant="glass">
        Selected glass card
      </Card>,
    );

    const card = screen.getByText("Selected glass card");

    expect(card).toHaveClass(surfaceStyles.glass);
    expect(card).not.toHaveClass(surfaceStyles.selected);
    expect(card).toHaveAttribute("data-card-variant", "glass");
    expect(card).toHaveAttribute("data-selected", "true");
  });

  it("renders GlassCard as a glass Card convenience", () => {
    renderCard(<GlassCard>Convenience card</GlassCard>);

    const card = screen.getByText("Convenience card");

    expect(card).toHaveClass(surfaceStyles.glass);
    expect(card).toHaveAttribute("data-card-variant", "glass");
  });
});
