import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { render } from "../../utils/test-utils";
import { setUpStore } from "../../app/store";
import { updateConfig } from "../Config/configSlice";
import { SplashScreen } from "./SplashScreen";

describe("startup diagnostics", () => {
  it.each(["Starting local Refact engine…", "Downloading Refact engine…"])(
    "opens diagnostics while %s with no engine",
    async (message) => {
      const store = setUpStore();
      store.dispatch(
        updateConfig({ host: "vscode", backendReady: false, lspPort: 0 }),
      );
      const { user } = render(<SplashScreen message={message} />, { store });
      await user.click(
        screen.getByRole("button", { name: "Open daemon diagnostics" }),
      );
      expect(store.getState().pages.at(-1)).toEqual({ name: "refact daemon" });
      await user.click(
        screen.getByRole("button", { name: "Component versions" }),
      );
      expect(screen.getByText("Plugin UI")).toBeInTheDocument();
      expect(screen.getByText("Project engine")).toBeInTheDocument();
    },
  );
});
