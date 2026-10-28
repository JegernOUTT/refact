import { describe, test, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { DiffContent } from "./DiffContent";
import groupBy from "lodash.groupby";

const STUB_DIFFS_1 = groupBy(
  [
    {
      file_name: "/emergency_frog_situation/frog.py",
      file_action: "edit",
      line1: 5,
      line2: 7,
      lines_remove: "class Frog:\n    def __init__(self, x, y, vx, vy):\n",
      lines_add: "class Bird:\n    def __init__(self, x, y, vx, vy):\n",
    },
    {
      file_name: "/emergency_frog_situation/frog.py",
      file_action: "edit",
      line1: 12,
      line2: 13,
      lines_remove:
        "    def bounce_off_banks(self, pond_width, pond_height):\n",
      lines_add: "    def bounce_off_banks(self, pond_width, pond_height):\n",
    },

    {
      file_name: "/emergency_frog_situation/frog.py",
      file_action: "edit",
      line1: 22,
      line2: 23,
      lines_remove: "    def jump(self, pond_width, pond_height):\n",
      lines_add: "    def jump(self, pond_width, pond_height):\n",
    },

    {
      file_name: "/emergency_frog_situation/holiday.py",
      file_action: "edit",
      line1: 1,
      line2: 2,
      lines_remove: "import frog\n",
      lines_add: "import frog as bird_module\n",
    },
    {
      file_name: "/emergency_frog_situation/holiday.py",
      file_action: "edit",
      line1: 5,
      line2: 7,
      lines_remove: "    frog1 = frog.Frog()\n    frog2 = frog.Frog()\n",
      lines_add:
        "    frog1 = bird_module.Bird()\n    frog2 = bird_module.Bird()\n",
    },
    {
      file_name: "/emergency_frog_situation/jump_to_conclusions.py",
      file_action: "edit",
      line1: 7,
      line2: 8,
      lines_remove: "import frog\n",
      lines_add: "import frog as bird_module\n",
    },
    {
      file_name: "/emergency_frog_situation/jump_to_conclusions.py",
      file_action: "edit",
      line1: 29,
      line2: 30,
      lines_remove: "    frog.Frog(\n",
      lines_add: "    bird_module.Bird(\n",
    },
    {
      file_name: "/emergency_frog_situation/jump_to_conclusions.py",
      file_action: "edit",
      line1: 50,
      line2: 51,
      lines_remove: "        p: frog.Frog\n",
      lines_add: "        p: bird_module.Bird\n",
    },
  ],
  (diff) => diff.file_name,
);

describe("diff message transcript item", () => {
  test("renders one open Changes row with files and +/- meta", () => {
    render(<DiffContent diffs={STUB_DIFFS_1} />);

    const item = screen.getByTestId("diff-content");
    expect(item).toHaveAttribute("data-open", "true");
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Changes");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "frog.py, holiday.py, jump_to_conclusions.py",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("+10 −10");
    expect(screen.getAllByTestId("transcript-toggle")).toHaveLength(1);
  });

  test("renders the rich diff blocks with file headers", () => {
    render(<DiffContent diffs={STUB_DIFFS_1} />);
    expect(screen.getAllByText("class Bird:").length).toBeGreaterThan(0);
    expect(
      screen.getAllByText("/emergency_frog_situation/frog.py").length,
    ).toBe(3);
  });

  test("collapses and reopens from the row", async () => {
    const { user } = render(<DiffContent diffs={STUB_DIFFS_1} />);
    const toggle = screen.getByTestId("transcript-toggle");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
  });
});
