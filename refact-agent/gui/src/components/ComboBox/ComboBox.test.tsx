import React from "react";
import { describe, test, vi, expect, afterEach } from "vitest";
import { render, cleanup, waitFor } from "../../utils/test-utils";
import { ComboBox, type ComboBoxProps } from "./ComboBox";
import { TextArea, type TextAreaProps } from "../TextArea";
import { useDebounceCallback } from "usehooks-ts";

const defaultCommands = ["@file ", "@workspace "];
const defaultArgs = ["/foo ", "/bar "];

const SlashApp = (props: Partial<ComboBoxProps>) => {
  const [value, setValue] = React.useState<string>(props.value ?? "");
  const [commands, setCommands] = React.useState<ComboBoxProps["commands"]>({
    completions: [],
    replace: [0, 0],
    is_cmd_executable: false,
  });

  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fakeRequestCommands = React.useCallback(
    useDebounceCallback(
      (query: string, cursor: number) => {
        if (query === "/" && cursor === 1) {
          setCommands({
            completions: ["/optimize", "/review", "/move"],
            completion_details: {
              "/optimize": {
                description: "Optimize code for performance",
                argument_hint: "[file-path]",
                source: "project_refact",
                kind: "cmd",
              },
              "/review": {
                description: "Review code for issues",
                source: "global_refact",
                kind: "skill",
              },
              "/move": {
                description: "Move text from one place to another",
                argument_hint: "<from> <to>",
                source: "project_refact",
                kind: "cmd",
              },
            },
            replace: [0, cursor],
            is_cmd_executable: false,
          });
          return;
        }

        if (query === "/opt" && cursor === 4) {
          setCommands({
            completions: ["/optimize"],
            completion_details: {
              "/optimize": {
                description: "Optimize code for performance",
                argument_hint: "[file-path]",
                source: "project_refact",
                kind: "cmd",
              },
            },
            replace: [0, cursor],
            is_cmd_executable: false,
          });
          return;
        }

        if (query === "@" && cursor === 1) {
          setCommands({
            completions: defaultCommands,
            replace: [0, cursor],
            is_cmd_executable: false,
          });
          return;
        }

        if (query === "@file " && cursor === 6) {
          setCommands({
            completions: defaultArgs,
            replace: [6, 6],
            is_cmd_executable: false,
          });
          return;
        }

        setCommands({
          completions: [],
          replace: [-1, -1],
          is_cmd_executable: false,
        });
      },
      0,
      { leading: true },
    ),
    [],
  );

  const defaultProps: ComboBoxProps = {
    commands,
    requestCommandsCompletion: fakeRequestCommands,
    onSubmit: () => ({}),
    value: value,
    onChange: setValue,
    placeholder: "Type @ or / for commands",
    render: (props: TextAreaProps) => <TextArea {...props} />,
    onHelpClick: () => ({}),
    ...props,
  };

  return <ComboBox {...defaultProps} />;
};

describe("ComboBox", () => {
  afterEach(cleanup);

  test("arrow down and Enter insert the command without submitting", async () => {
    const onSubmitSpy = vi.fn();
    const { user, ...app } = render(<SlashApp onSubmit={onSubmitSpy} />);
    const textarea = app.getByRole("combobox");
    await user.type(textarea, "/");
    await user.keyboard("{ArrowDown}{Enter}");
    await waitFor(() => expect(textarea.textContent).toEqual("/review "));
    expect(onSubmitSpy).not.toHaveBeenCalled();
  });

  test("Enter on a command with arguments inserts the hint and selects the placeholder", async () => {
    const { user, ...app } = render(<SlashApp />);
    const textarea = app.getByRole("combobox") as HTMLTextAreaElement;
    await user.type(textarea, "/");
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(textarea.textContent).toEqual("/optimize [file-path]");
      expect(textarea.selectionStart).toEqual(10);
      expect(textarea.selectionEnd).toEqual(21);
    });
  });

  test("Escape closes the list", async () => {
    const { user, ...app } = render(<SlashApp />);
    await user.type(app.getByRole("combobox"), "/");
    expect(app.queryByText("/optimize")).not.toBeNull();
    await user.keyboard("{Escape}");
    expect(app.queryByText("/optimize")).toBeNull();
  });

  test("@ completes the command and then its argument", async () => {
    const { user, ...app } = render(<SlashApp />);
    const textarea = app.getByRole("combobox");
    await user.type(textarea, "@");
    await user.click(app.getByText("@file"));
    await waitFor(() => expect(textarea.textContent).toEqual("@file "));
    await user.click(await app.findByText("/bar"));
    expect(textarea.textContent).toEqual("@file /bar ");
  });
});
