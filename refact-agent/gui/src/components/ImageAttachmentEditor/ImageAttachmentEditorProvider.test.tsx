import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ImageAttachmentEditorProvider } from "./ImageAttachmentEditorProvider";
import { ImageAttachmentEditorContext } from "./context";

vi.hoisted(() => vi.resetModules());
const mocks = vi.hoisted(() => ({
  scope: { key: "chat-a", allowed: true },
  dispatch: vi.fn(),
}));
vi.mock("./useImageAttachmentScope", () => ({
  useImageAttachmentScope: () => mocks.scope,
}));
vi.mock("../../hooks/useAppDispatch", () => ({
  useAppDispatch: () => mocks.dispatch,
}));
vi.mock("../ImageEditor", () => ({
  ImageEditor: ({
    file,
    onApply,
    onCancel,
  }: {
    file: File;
    onApply: (file: File) => void;
    onCancel: () => void;
  }) => (
    <div role="dialog">
      {file.name}
      <button
        onClick={() =>
          onApply(new File(["edited"], "edited.png", { type: "image/png" }))
        }
      >
        Apply
      </button>
      <button onClick={onCancel}>Cancel</button>
    </div>
  ),
}));

function Ingress({ accept }: { accept: (file: File) => void }) {
  const enqueue = React.useContext(ImageAttachmentEditorContext);
  return (
    <button
      onClick={() =>
        enqueue?.(
          ["first.png", "second.png"].map((name) => ({
            file: new File(["raw"], name, { type: "image/png" }),
            accept,
          })),
        )
      }
    >
      Add
    </button>
  );
}

describe("ImageAttachmentEditorProvider", () => {
  beforeEach(() => {
    mocks.scope = { key: "chat-a", allowed: true };
    mocks.dispatch.mockClear();
  });
  it("shares one sequential queue across ingress consumers and only attaches Apply", () => {
    const accept = vi.fn<(file: File) => void>();
    render(
      <ImageAttachmentEditorProvider>
        <Ingress accept={accept} />
        <Ingress accept={accept} />
      </ImageAttachmentEditorProvider>,
    );
    fireEvent.click(screen.getAllByText("Add")[0]);
    fireEvent.click(screen.getAllByText("Add")[1]);
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getByRole("dialog")).toHaveTextContent("first.png");
    expect(accept).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Cancel"));
    expect(screen.getByRole("dialog")).toHaveTextContent("second.png");
    fireEvent.click(screen.getByText("Apply"));
    expect(accept).toHaveBeenCalledTimes(1);
    expect(accept.mock.calls[0][0].name).toBe("edited.png");
    expect(screen.getByRole("dialog")).toHaveTextContent("first.png");
  });
  it("discards queued drafts on thread, endpoint, model or capability scope changes", () => {
    const accept = vi.fn<(file: File) => void>();
    const tree = () => (
      <ImageAttachmentEditorProvider>
        <Ingress accept={accept} />
      </ImageAttachmentEditorProvider>
    );
    const view = render(tree());
    fireEvent.click(screen.getByText("Add"));
    mocks.scope = { key: "chat-b", allowed: true };
    view.rerender(tree());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("Add"));
    mocks.scope = { key: "denied", allowed: false };
    view.rerender(tree());
    fireEvent.click(screen.getByText("Add"));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(accept).not.toHaveBeenCalled();
  });
  it("bounds pending images across batches", () => {
    const accept = vi.fn<(file: File) => void>();
    render(
      <ImageAttachmentEditorProvider>
        <Ingress accept={accept} />
      </ImageAttachmentEditorProvider>,
    );
    for (let i = 0; i < 26; i++) fireEvent.click(screen.getByText("Add"));
    expect(mocks.dispatch).toHaveBeenCalledTimes(1);
    expect(accept).not.toHaveBeenCalled();
  });
});
