import { describe, expect, it, vi } from "vitest";

import { setUpStore } from "../../../app/store";
import { CapsProvidersErrorBanner } from "../../Errors/CapsProvidersErrorBanner";
import { setError, type ErrorPayload } from "../../Errors/errorsSlice";
import { render, screen } from "../../../utils/test-utils";
import { ProvidersView } from "./ProvidersView";

vi.mock("./ConfiguredProvidersView", () => ({
  ConfiguredProvidersView: () => null,
}));

vi.mock("./AddProviderInstanceModal", () => ({
  AddProviderInstanceModal: () => null,
}));

vi.mock("../../DefaultModels", () => ({
  DefaultModels: () => null,
}));

const CAPS_ERROR = "fetching caps from lsp";
const ROUTE_ERROR = "provider update error.";

function renderProvidersRoute(...errors: ErrorPayload[]) {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  errors.forEach((error) => store.dispatch(setError(error)));
  return render(
    <>
      <CapsProvidersErrorBanner onRetry={() => void 0} isRetrying={false} />
      <ProvidersView
        configuredProviders={[]}
        backFromProviders={() => void 0}
      />
    </>,
    { store },
  );
}

describe("Providers route error de-duplication", () => {
  it("renders a caps/providers failure exactly once", () => {
    renderProvidersRoute({ message: CAPS_ERROR, source: "caps_providers" });

    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getAllByText(CAPS_ERROR)).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("still renders route-local errors in the providers callout", () => {
    renderProvidersRoute(ROUTE_ERROR);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(document.body).toHaveTextContent(ROUTE_ERROR);
  });

  it("renders a route error raised while the caps banner is shown", () => {
    renderProvidersRoute(
      { message: CAPS_ERROR, source: "caps_providers" },
      ROUTE_ERROR,
    );

    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getAllByText(CAPS_ERROR)).toHaveLength(1);
    expect(document.body).toHaveTextContent(ROUTE_ERROR);
  });
});
