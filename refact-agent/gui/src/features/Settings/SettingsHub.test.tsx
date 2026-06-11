import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { reducer as configReducer } from "../Config/configSlice";
import { pagesSlice, type Page } from "../Pages/pagesSlice";
import { SettingsHub } from "./SettingsHub";

vi.mock("../Providers", () => ({
  Providers: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="providers-section" data-embedded={String(embedded)} />
  ),
}));

vi.mock("../DefaultModels", () => ({
  DefaultModels: ({
    embedded,
    draftId,
  }: {
    embedded?: boolean;
    draftId?: string;
  }) => (
    <div
      data-testid="default-models-section"
      data-embedded={String(embedded)}
      data-draft-id={draftId}
    />
  ),
}));

vi.mock("../Customization", () => ({
  Customization: ({
    embedded,
    initialKind,
    initialConfigId,
    draftId,
  }: {
    embedded?: boolean;
    initialKind?: string;
    initialConfigId?: string;
    draftId?: string;
  }) => (
    <div
      data-testid="customization-section"
      data-embedded={String(embedded)}
      data-kind={initialKind}
      data-config-id={initialConfigId}
      data-draft-id={draftId}
    />
  ),
}));

vi.mock("../Integrations", () => ({
  Integrations: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="integrations-section" data-embedded={String(embedded)} />
  ),
}));

vi.mock("../Scheduler", () => ({
  SchedulerPanel: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="scheduler-section" data-embedded={String(embedded)} />
  ),
}));

vi.mock("../MarketplaceHub", () => ({
  MarketplaceHub: ({ embedded }: { embedded?: boolean }) => (
    <div data-testid="marketplace-section" data-embedded={String(embedded)} />
  ),
}));

vi.mock("./GeneralSettingsSection", () => ({
  GeneralSettingsSection: () => <div data-testid="general-section" />,
}));

function createTestStore(extraPages: Page[] = []) {
  return configureStore({
    reducer: {
      config: configReducer,
      pages: pagesSlice.reducer,
    },
    preloadedState: {
      pages: [{ name: "login page" }, ...extraPages] as Page[],
    },
  });
}

function renderHub(page: Page, onBack = vi.fn()) {
  const store = createTestStore([page]);
  render(
    <Provider store={store}>
      <SettingsHub page={page} onBack={onBack} host="web" tabbed={false} />
    </Provider>,
  );
  return { store, onBack };
}

describe("SettingsHub — section routing by page name", () => {
  it("shows Providers section for providers page", () => {
    renderHub({ name: "providers page" });
    expect(screen.getByTestId("providers-section")).toBeInTheDocument();
  });

  it("shows DefaultModels section for default models page", () => {
    renderHub({ name: "default models" });
    expect(screen.getByTestId("default-models-section")).toBeInTheDocument();
  });

  it("forwards draftId to DefaultModels when page has draftId", () => {
    renderHub({ name: "default models", draftId: "draft-123" });
    expect(screen.getByTestId("default-models-section")).toHaveAttribute(
      "data-draft-id",
      "draft-123",
    );
  });

  it("shows Customization section with kind and configId forwarded", () => {
    renderHub({ name: "customization", kind: "modes", configId: "my_mode" });
    const el = screen.getByTestId("customization-section");
    expect(el).toBeInTheDocument();
    expect(el).toHaveAttribute("data-kind", "modes");
    expect(el).toHaveAttribute("data-config-id", "my_mode");
  });

  it("shows Integrations section for integrations page", () => {
    renderHub({ name: "integrations page" });
    expect(screen.getByTestId("integrations-section")).toBeInTheDocument();
  });

  it("shows Scheduler section for scheduler page", () => {
    renderHub({ name: "scheduler" });
    expect(screen.getByTestId("scheduler-section")).toBeInTheDocument();
  });

  it("shows General section for general settings page", () => {
    renderHub({ name: "general settings" });
    expect(screen.getByTestId("general-section")).toBeInTheDocument();
  });

  it("shows Marketplace section for marketplace hub page", () => {
    renderHub({ name: "marketplace hub" });
    const section = screen.getByTestId("marketplace-section");
    expect(section).toBeInTheDocument();
    expect(section).toHaveAttribute("data-embedded", "true");
  });
});

describe("SettingsHub — left nav dispatches change(), not push()", () => {
  it("switches to providers section via change (stack length unchanged)", () => {
    const { store } = renderHub({ name: "general settings" });
    const initialLength = store.getState().pages.length;

    fireEvent.click(screen.getByRole("button", { name: "Providers" }));

    const pages = store.getState().pages;
    expect(pages.length).toBe(initialLength);
    expect(pages[pages.length - 1].name).toBe("providers page");
  });

  it("switches to models section via change (stack length unchanged)", () => {
    const { store } = renderHub({ name: "general settings" });
    const initialLength = store.getState().pages.length;

    fireEvent.click(screen.getByRole("button", { name: "Models" }));

    const pages = store.getState().pages;
    expect(pages.length).toBe(initialLength);
    expect(pages[pages.length - 1].name).toBe("default models");
  });

  it("switches to marketplace section via change (stack length unchanged)", () => {
    const { store } = renderHub({ name: "general settings" });
    const initialLength = store.getState().pages.length;

    fireEvent.click(screen.getByRole("button", { name: "Marketplace" }));

    const pages = store.getState().pages;
    expect(pages.length).toBe(initialLength);
    expect(pages[pages.length - 1].name).toBe("marketplace hub");
  });
});

describe("SettingsHub — Back button calls onBack", () => {
  it("calls onBack when Back is clicked", () => {
    const onBack = vi.fn();
    renderHub({ name: "general settings" }, onBack);

    const backButton = screen.getByText("Back");
    fireEvent.click(backButton);

    expect(onBack).toHaveBeenCalledOnce();
  });
});

describe("SettingsHub — embedded integrations section", () => {
  it("passes embedded=true to Integrations so section has no inner Back button", () => {
    renderHub({ name: "integrations page" });
    const section = screen.getByTestId("integrations-section");
    expect(section).toHaveAttribute("data-embedded", "true");
    expect(screen.getAllByRole("button", { name: /back/i })).toHaveLength(1);
  });

  it("routes integrations page with integrationPath to integrations section", () => {
    renderHub({
      name: "integrations page",
      integrationPath: "/some/config.yaml",
    });
    expect(screen.getByTestId("integrations-section")).toBeInTheDocument();
    expect(screen.getByTestId("integrations-section")).toHaveAttribute(
      "data-embedded",
      "true",
    );
  });
});
