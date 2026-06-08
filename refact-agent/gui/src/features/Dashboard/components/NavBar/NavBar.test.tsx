import { describe, it, expect } from "vitest";
import userEvent from "@testing-library/user-event";
import { render, screen } from "../../../../utils/test-utils";
import { NavBar } from "./NavBar";

function renderNavBar() {
  return render(<NavBar />);
}

describe("NavBar", () => {
  it("clicking Settings dispatches push({name:'general settings'})", async () => {
    const { store } = renderNavBar();

    await userEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(store.getState().pages.at(-1)?.name).toBe("general settings");
  });

  it("clicking Stats dispatches push({name:'stats dashboard'})", async () => {
    const { store } = renderNavBar();

    await userEvent.click(screen.getByRole("button", { name: "Stats" }));

    expect(store.getState().pages.at(-1)?.name).toBe("stats dashboard");
  });

  it("clicking Marketplace dispatches push({name:'marketplace hub'})", async () => {
    const { store } = renderNavBar();

    await userEvent.click(screen.getByRole("button", { name: "Marketplace" }));

    expect(store.getState().pages.at(-1)?.name).toBe("marketplace hub");
  });

  it("does not show the removed settings cards", () => {
    renderNavBar();

    expect(screen.queryByRole("button", { name: "Integrations" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Providers" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Modes" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Scheduler" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Extensions" })).not.toBeInTheDocument();
  });
});
