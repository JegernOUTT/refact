import { describe, expect, it } from "vitest";
import { render, screen } from "../utils/test-utils";
import { VirtualizedGrid } from "../components/ui/VirtualizedGrid";

type Item = { id: string; label: string };

const makeItems = (count: number): Item[] =>
  Array.from({ length: count }, (_, index) => ({
    id: `item-${index}`,
    label: `Item ${index}`,
  }));

describe("VirtualizedGrid", () => {
  it("renders all items in plain mode for small lists", () => {
    render(
      <VirtualizedGrid
        items={makeItems(5)}
        getItemKey={(item) => item.id}
        renderItem={(item) => <span>{item.label}</span>}
      />,
    );
    expect(screen.getByText("Item 0")).toBeInTheDocument();
    expect(screen.getByText("Item 4")).toBeInTheDocument();
  });

  it("renders items in virtualized mode once the threshold is exceeded", () => {
    render(
      <VirtualizedGrid
        items={makeItems(120)}
        virtualizeThreshold={80}
        getItemKey={(item) => item.id}
        renderItem={(item) => <span>{item.label}</span>}
      />,
    );
    expect(screen.getByText("Item 0")).toBeInTheDocument();
    expect(screen.getByText("Item 119")).toBeInTheDocument();
  });

  it("renders a single column when columns is 1", () => {
    render(
      <VirtualizedGrid
        items={makeItems(3)}
        columns={1}
        getItemKey={(item) => item.id}
        renderItem={(item) => <span>{item.label}</span>}
      />,
    );
    expect(screen.getByText("Item 2")).toBeInTheDocument();
  });

  it("renders nothing when there are no items", () => {
    const { container } = render(
      <VirtualizedGrid
        items={[]}
        renderItem={(item: Item) => <span>{item.label}</span>}
      />,
    );
    expect(container.querySelector("span")).toBeNull();
  });
});
