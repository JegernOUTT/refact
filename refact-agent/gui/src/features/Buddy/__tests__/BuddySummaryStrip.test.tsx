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

  it("hides the XP bar at max stage and keeps View stats with the stats", () => {
    render(
      <BuddySummaryStrip
        name="Pixel"
        palette={PALETTES[0]}
        stage={STAGES[STAGES.length - 1]}
        stageIndex={STAGES.length - 1}
        xp={210}
        xpNext={undefined}
        xpFill={100}
        atMaxStage
        pet={undefined}
        statsData={{
          totals: { total_calls: 10, successful_calls: 9, total_tokens: 100 },
        }}
        successRate={90}
        onViewStats={vi.fn()}
      />,
    );

    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.getByText(/max stage/)).toBeInTheDocument();
    const viewStats = screen.getByRole("button", { name: "View stats" });
    expect(viewStats.parentElement).toBe(
      screen.getByText("Messages").closest("div")?.parentElement,
    );
  });

  it("shows XP progress below max stage", () => {
    render(
      <BuddySummaryStrip
        name="Pixel"
        palette={PALETTES[0]}
        stage={STAGES[0]}
        stageIndex={0}
        xp={5}
        xpNext={20}
        xpFill={25}
        atMaxStage={false}
        pet={undefined}
        statsData={undefined}
        successRate={null}
        onViewStats={vi.fn()}
      />,
    );

    const progress = screen.getByRole("progressbar", {
      name: "Progress to next stage",
    });
    expect(progress).toHaveAttribute("value", "25");
    expect(progress).toHaveAttribute("max", "100");
    expect(progress.tagName).toBe("PROGRESS");
  });
});
