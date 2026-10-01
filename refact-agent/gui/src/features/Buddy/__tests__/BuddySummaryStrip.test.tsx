import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { PALETTES, STAGES } from "../constants";
import { BuddySummaryStrip } from "../BuddySummaryStrip";

describe("BuddySummaryStrip", () => {
  it("shows care but not the neglect score", () => {
    render(
      <BuddySummaryStrip
        name="Buddy"
        palette={PALETTES[0]}
        stage={STAGES[0]}
        stageIndex={0}
        xp={0}
        xpNext={20}
        xpFill={0}
        atMaxStage={false}
        pet={{
          needs: {
            hunger: 80,
            energy: 85,
            hygiene: 80,
            boredom: 15,
            affection: 75,
          },
          condition: {
            sleeping: false,
            hungry: false,
            sleepy: false,
            dirty: false,
            bored: false,
            lonely: false,
          },
          evolution: {
            care_score: 4,
            neglect_score: 83,
            open_seconds: 0,
            last_evolved_at: null,
          },
        }}
        statsData={undefined}
        successRate={null}
        onViewStats={vi.fn()}
      />,
    );

    expect(screen.getByText("Care")).toBeInTheDocument();
    expect(screen.getByText("4")).toBeInTheDocument();
    expect(screen.queryByText("Neglect")).not.toBeInTheDocument();
    expect(screen.queryByText("83")).not.toBeInTheDocument();
  });
});
