import { describe, test, vi, expect } from "vitest";
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

describe("diff content disclosure", () => {
  test("renders a semantic trigger with aria state and controls", () => {
    render(<DiffContent diffs={STUB_DIFFS_1} />);

    const trigger = screen.getByRole("button", {
      name: /frog\.py \+7 -7 , holiday\.py \+5 -5 , jump_to_conclusions\.py \+6 -6/i,
    });
    const content = document.getElementById(
      trigger.getAttribute("aria-controls") ?? "",
    );

    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(content).toBeTruthy();
    expect(content).toHaveAttribute("data-open", "false");
  });

  test("toggles with mouse and keyboard", async () => {
    const { user } = render(<DiffContent diffs={STUB_DIFFS_1} />);
    const trigger = screen.getByRole("button", { name: /frog\.py/i });
    const content = document.getElementById(
      trigger.getAttribute("aria-controls") ?? "",
    );

    await user.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(content).toHaveAttribute("data-open", "true");

    trigger.focus();
    await user.keyboard("{Enter}");

    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(content).toHaveAttribute("data-open", "false");

    await user.keyboard(" ");

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(content).toHaveAttribute("data-open", "true");
  });
});

// TODO: mock requests with msw when chat has been migrated.
describe.skip("diff content", () => {
  test("apply all, none applied", async () => {
    const onSumbitSpy = vi.fn();
    const { user, ...app } = render(<DiffContent diffs={STUB_DIFFS_1} />);

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(app.container.querySelector('[type="button"]')!);
    const btn = app.getByText(/Apply all/i);
    await user.click(btn);
    expect(onSumbitSpy).toHaveBeenCalledWith([
      true,
      true,
      true,
      true,
      true,
      true,
      true,
      true,
    ]);
  });
  test("apply all", async () => {
    const onSumbitSpy = vi.fn();
    const { user, ...app } = render(<DiffContent diffs={STUB_DIFFS_1} />);

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(app.container.querySelector('[type="button"]')!);
    const btn = app.getByText(/Apply all/i);
    await user.click(btn);
    expect(onSumbitSpy).toHaveBeenCalledWith([
      true,
      true,
      true,
      true,
      true,
      true,
      true,
      true,
    ]);
  });

  test("unapply all", async () => {
    const onSumbitSpy = vi.fn();
    const { user, ...app } = render(<DiffContent diffs={STUB_DIFFS_1} />);

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(app.container.querySelector('[type="button"]')!);
    const btn = app.getByText(/unapply all/i);
    await user.click(btn);
    expect(onSumbitSpy).toHaveBeenCalledWith([
      false,
      false,
      false,
      false,
      false,
      false,
      false,
      false,
    ]);
  });

  test("disable apply all", async () => {
    const onSumbitSpy = vi.fn();
    const { user, ...app } = render(<DiffContent diffs={STUB_DIFFS_1} />);

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(app.container.querySelector('[type="button"]')!);
    const btn = app.getByText(/apply all/i) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    await user.click(btn);
    expect(onSumbitSpy).not.toHaveBeenCalled();
  });

  test("apply individual file", async () => {
    const onSumbitSpy = vi.fn();
    const { user, ...app } = render(<DiffContent diffs={STUB_DIFFS_1} />);

    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    await user.click(app.container.querySelector('[type="button"]')!);
    const btns = app.getAllByText(/apply/i);
    await user.click(btns[0]);
    expect(onSumbitSpy).toHaveBeenCalledWith([
      true,
      true,
      true,
      false,
      false,
      false,
      false,
      false,
    ]);

    app.rerender(<DiffContent diffs={STUB_DIFFS_1} />);

    expect(() => app.queryByText(/applied/i)).not.toBeNull();
  });
});
