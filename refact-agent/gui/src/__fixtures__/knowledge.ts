import type { MemoRecord, VecDbStatus } from "../services/refact";

export const STUB_MEMORIES: MemoRecord[] = [
  {
    memid: "7666487b81",
    tags: ["rust", "compilation"],
    content: "Looks like proj1 is written in fact in Rust.",
    file_path: ".refact/knowledge/proj1-rust-fact.md",
    title: "Project Language Detection",
    created: "2024-01-15T10:30:00Z",
    kind: "proj-fact",
    score: 0.95,
  },
  {
    memid: "cdec854819",
    tags: ["rust", "build", "success"],
    content: "Wow, running cargo build on proj2 was successful!",
    file_path: ".refact/knowledge/proj2-build-success.md",
    title: "Successful Build",
    created: "2024-01-15T11:00:00Z",
    kind: "seq-of-acts",
    score: 0.88,
  },
  {
    memid: "eb1d64684b",
    tags: ["rust", "language"],
    content: "Looks like proj2 is written in fact in Rust.",
    file_path: ".refact/knowledge/proj2-rust-fact.md",
    title: "Project Language",
    created: "2024-01-15T11:30:00Z",
    kind: "proj-fact",
    score: 0.92,
  },
  {
    memid: "eb1d64684c",
    tags: ["rust", "long-description"],
    content: "Looks like proj2 is written in fact in Rust.",
    file_path: ".refact/knowledge/proj2-long-goal.md",
    title: "Long goal Long goal Long goal Long goal Long goal Long goal Long goal Long goal Long goal Long goal",
    created: "2024-01-15T12:00:00Z",
    kind: "proj-fact",
    score: 0.85,
  },
];

// export const STUB_SUB_RESPONSE: MemdbSubEvent[] = [
//   {
//     pubevent_id: 19,
//     pubevent_action: "INSERT",
//     pubevent_json: {
//       memid: "66a072d699",
//       m_type: "seq-of-acts",
//       m_goal: "compile",
//       m_project: "proj1",
//       m_payload: "Wow, running cargo build on proj1 was successful!",
//       m_origin: "local-committed",
//       mstat_correct: 0.0,
//       mstat_relevant: 0.0,
//       mstat_times_used: 0,
//     },
//   },
//   {
//     pubevent_id: 26,
//     pubevent_action: "INSERT",
//     pubevent_json: {
//       memid: "d688925823",
//       m_type: "proj-fact",
//       m_goal: "compile",
//       m_project: "proj1",
//       m_payload: "Looks like proj1 is written in fact in Rust.",
//       m_origin: "local-committed",
//       mstat_correct: 0.0,
//       mstat_relevant: 0.0,
//       mstat_times_used: 0,
//     },
//   },
//   {
//     pubevent_id: 27,
//     pubevent_action: "INSERT",
//     pubevent_json: {
//       memid: "08f9374753",
//       m_type: "seq-of-acts",
//       m_goal: "compile",
//       m_project: "proj2",
//       m_payload: "Wow, running cargo build on proj2 was successful!",
//       m_origin: "local-committed",
//       mstat_correct: 0.0,
//       mstat_relevant: 0.0,
//       mstat_times_used: 0,
//     },
//   },
//   {
//     pubevent_id: 28,
//     pubevent_action: "INSERT",
//     pubevent_json: {
//       memid: "c9cefe3ff4",
//       m_type: "proj-fact",
//       m_goal: "compile",
//       m_project: "proj2",
//       m_payload: "Looks like proj2 is written in fact in Rust.",
//       m_origin: "local-committed",
//       mstat_correct: 0.0,
//       mstat_relevant: 0.0,
//       mstat_times_used: 0,
//     },
//   },
//   {
//     pubevent_id: 29,
//     pubevent_action: "UPDATE",
//     pubevent_json: {
//       memid: "d688925823",
//       m_type: "proj-fact",
//       m_goal: "compile",
//       m_project: "proj1",
//       m_payload: "Looks like proj1 is written in fact in Rust.",
//       m_origin: "local-committed",
//       mstat_correct: 1.0,
//       mstat_relevant: -1.0,
//       mstat_times_used: 1,
//     },
//   },
//   {
//     pubevent_id: 30,
//     pubevent_action: "DELETE",
//     pubevent_json: {
//       memid: "9d2a679b09",
//       m_type: "",
//       m_goal: "",
//       m_project: "",
//       m_payload: "",
//       m_origin: "",
//       mstat_correct: 0,
//       mstat_relevant: 0,
//       mstat_times_used: 0,
//     },
//   },
// ];

// export const STUB_SUB_RESPONSE_WITH_STATUS: (
//   | MemdbSubEventUnparsed
//   | VecDbStatus
// )[] = [];

export const VECDB_STATUS_STARTING: VecDbStatus = {
  files_unprocessed: 0,
  files_total: 0,
  requests_made_since_start: 1,
  vectors_made_since_start: 33,
  db_size: 33,
  db_cache_size: 37,
  state: "starting",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDB_STATUS_PARSING: VecDbStatus = {
  files_unprocessed: 377,
  files_total: 404,
  requests_made_since_start: 5,
  vectors_made_since_start: 296,
  db_size: 168,
  db_cache_size: 333,
  state: "parsing",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDB_STATUS_PARSING_2: VecDbStatus = {
  files_unprocessed: 372,
  files_total: 404,
  requests_made_since_start: 6,
  vectors_made_since_start: 303,
  db_size: 303,
  db_cache_size: 340,
  state: "parsing",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDV_STATUS_PARISING_3: VecDbStatus = {
  files_unprocessed: 192,
  files_total: 404,
  requests_made_since_start: 21,
  vectors_made_since_start: 990,
  db_size: 1021,
  db_cache_size: 1027,
  state: "parsing",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDB_STATUS_PARSING_4: VecDbStatus = {
  files_unprocessed: 12,
  files_total: 404,
  requests_made_since_start: 52,
  vectors_made_since_start: 2494,
  db_size: 2524,
  db_cache_size: 2531,
  state: "parsing",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDB_STATUS_COOLDOWN: VecDbStatus = {
  files_unprocessed: 1,
  files_total: 404,
  requests_made_since_start: 52,
  vectors_made_since_start: 2494,
  db_size: 2524,
  db_cache_size: 2533,
  state: "cooldown",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const VECDB_STATUS_DONE: VecDbStatus = {
  files_unprocessed: 0,
  files_total: 0,
  requests_made_since_start: 54,
  vectors_made_since_start: 2535,
  db_size: 2629,
  db_cache_size: 2574,
  state: "done",
  queue_additions: false,
  vecdb_max_files_hit: false,
  vecdb_errors: {},
};

export const STUB_SUB_RESPONSE_WITH_STATUS = [
  VECDB_STATUS_STARTING,
  // ...STUB_SUB_RESPONSE,
  VECDB_STATUS_PARSING,
  VECDB_STATUS_PARSING_2,
  VECDV_STATUS_PARISING_3,
  VECDB_STATUS_PARSING_4,
  VECDB_STATUS_COOLDOWN,
  VECDB_STATUS_DONE,
];

export const STB_LOADING_VECDB = {
  VECDB_STATUS_STARTING,
  VECDB_STATUS_PARSING,
  VECDB_STATUS_PARSING_2,
  VECDV_STATUS_PARISING_3,
  VECDB_STATUS_PARSING_4,
  VECDB_STATUS_COOLDOWN,
  VECDB_STATUS_DONE,
};
