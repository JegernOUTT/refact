import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { TaskList } from "../features/Tasks/TaskList";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

describe("TaskCreate", () => {
  it("task_create_form_accepts_target_files_input", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks", () => HttpResponse.json([])),
    );

    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });

    await user.click(await screen.findByRole("button", { name: /new task/i }));
    const targetFilesInput = screen.getByLabelText("Target files");
    await user.type(targetFilesInput, "src/foo.rs, src/bar.ts");

    expect(targetFilesInput).toHaveValue("src/foo.rs, src/bar.ts");
  });

  it("task_create_form_posts_target_files_array", async () => {
    let postedBody: unknown;
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks", () => HttpResponse.json([])),
      http.post("http://127.0.0.1:8001/v1/tasks", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({
          id: "task-1",
          name: "New task",
          status: "planning",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
          cards_total: 0,
          cards_done: 0,
          cards_failed: 0,
          agents_active: 0,
        });
      }),
    );

    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });

    await user.click(await screen.findByRole("button", { name: /new task/i }));
    await user.type(screen.getByPlaceholderText("Task name..."), "New task");
    await user.type(
      screen.getByLabelText("Target files"),
      "src/foo.rs, src/bar.ts",
    );
    await user.click(screen.getByRole("button", { name: /^create$/i }));

    await waitFor(() => {
      expect(postedBody).toEqual({
        name: "New task",
        target_files: ["src/foo.rs", "src/bar.ts"],
      });
    });
  });
});
