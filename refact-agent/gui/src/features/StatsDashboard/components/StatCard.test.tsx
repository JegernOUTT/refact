import { describe, expect, it } from "vitest";
import { Coins } from "lucide-react";
import { render, screen } from "../../../utils/test-utils";
import { StatCard } from "./StatCard";

describe("StatCard", () => {
  it("renders usage stat content with kit styling hooks", () => {
    render(
      <StatCard
        icon={Coins}
        title="Total Cost"
        value="$1.23"
        subtitle="across all providers"
        tone="warning"
      />,
    );

    expect(screen.getByText("Total Cost")).toBeInTheDocument();
    expect(screen.getByText("$1.23")).toBeInTheDocument();
    expect(screen.getByText("across all providers")).toBeInTheDocument();
  });
});
