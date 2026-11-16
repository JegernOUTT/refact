// Real wire messages copied from a lab engine's /v1/trajectories/<id>.
// The engine flattens ChatMessage.extra, so `python` sits at the top level.
// Do not hand-edit the shape: these pin the engine's serialization.

export const flattenedPythonToolMessage = {
  message_id: "f0b0ba46-8b71-4e4a-9a4e-bd97fd92e15c",
  role: "tool",
  content:
    'Printed output:\nbefore\n\n\nError: ValueError: boom\nTraceback (most recent call last):\n  File "<python-input-0>", line 1, in <module>\n    print("before"); raise ValueError("boom")\nValueError: boom',
  tool_call_id: "call_6b79eeb6881840d29c2b7790ab3e79ff",
  tool_failed: true,
  privacy: {
    files: [
      {
        path: "/tmp/pycm-e2e/project/README.md",
        zone: "normal",
        attribution: "declared",
      },
    ],
  },
  python: {
    schema: 1,
    run_id: "9d6c4c63",
    status: "failed",
    background: false,
    started_at_ms: 1794684219269,
    finished_at_ms: 1794684219289,
    workspace_dir:
      "/tmp/pycm-e2e/project/.refact/trajectories/d8e732c8-f0d1-4d7a-92ac-3896b7547954/python/d8e732c8-f0d1-4d7a-92ac-3896b7547954",
    run_dir:
      "/tmp/pycm-e2e/project/.refact/trajectories/d8e732c8-f0d1-4d7a-92ac-3896b7547954/python/d8e732c8-f0d1-4d7a-92ac-3896b7547954/runs/9d6c4c63",
    calls: [],
    calls_total: 0,
    calls_truncated: false,
    stdout_preview: "before\n",
    stdout_bytes: 7,
    result_preview: "",
    result_bytes: 0,
    error: {
      type: "ValueError",
      message: "boom",
      traceback:
        'Traceback (most recent call last):\n  File "<python-input-0>", line 1, in <module>\n    print("before"); raise ValueError("boom")\nValueError: boom',
    },
    files_read: [],
    files_written: [],
    artifacts: [],
    state: {
      restored: false,
      reset_reason: null,
      persisted: true,
      bytes: 8162,
    },
  },
  tool_job_postprocessed: true,
};

export const flattenedPythonDeliveryEvent = {
  message_id: "tool_job:tooljob_9a3732d8-57a3-4298-8fda-e77311366241:2:0",
  role: "event",
  content:
    "Result:\nnull\n\nPrinted output:\ndone\n\n\nNested tool calls:\n- shell: ok (25214 ms) Nothing in STDOUT/STDERR\nState: restored from disk",
  tool_failed: false,
  privacy: {
    files: [],
  },
  python: {
    schema: 1,
    run_id: "03c35f4e",
    status: "completed",
    background: true,
    started_at_ms: 1794684642449,
    finished_at_ms: 1794684667736,
    workspace_dir:
      "/tmp/pycm-e2e/project/.refact/trajectories/f06a4bfc-080f-4380-ac17-5e76ae875c7e/python/f06a4bfc-080f-4380-ac17-5e76ae875c7e",
    run_dir:
      "/tmp/pycm-e2e/project/.refact/trajectories/f06a4bfc-080f-4380-ac17-5e76ae875c7e/python/f06a4bfc-080f-4380-ac17-5e76ae875c7e/runs/03c35f4e",
    calls: [
      {
        seq: 1,
        call_id: "py_03c35f4e_1",
        tool: "shell",
        args_preview:
          '{"command":"sleep 25","description":"Screenshot background probe"}',
        status: "ok",
        summary: "Nothing in STDOUT/STDERR",
        started_at_ms: 1794684642455,
        duration_ms: 25214,
        approval: "not_needed",
        diffs: [],
      },
    ],
    calls_total: 1,
    calls_truncated: false,
    stdout_preview: "done\n",
    stdout_bytes: 5,
    result_preview: "null",
    result_bytes: 4,
    error: null,
    files_read: [],
    files_written: [],
    artifacts: [],
    state: {
      restored: true,
      reset_reason: null,
      persisted: true,
      bytes: 11616,
    },
  },
  tool_job_postprocessed: true,
  tool_job_result: {
    job_id: "tooljob_9a3732d8-57a3-4298-8fda-e77311366241",
    revision: 2,
    tool_call_id: "call_afd6ea3a73f84e448d33da4208ccaf0c",
    original_role: "tool",
    actor: "tool_completion",
    tool_name: "python",
  },
  event: {
    subkind: "system_notice",
    source: "tool_job.completed",
    payload: {
      actor: "tool_completion",
      job_id: "tooljob_9a3732d8-57a3-4298-8fda-e77311366241",
    },
  },
  delivery: {
    id: "tool_job:tooljob_9a3732d8-57a3-4298-8fda-e77311366241:2",
    source: "tool_job.completed",
    push: "append",
    at_ms: 1794684667743,
  },
};
