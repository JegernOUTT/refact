import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import type {
  CodeGraphStatus,
  MemoRecord,
  RagStatus,
  VecDbStatus,
} from "../../services/refact/types";

export type KnowledgeState = {
  loaded: boolean;
  memories: Record<string, MemoRecord>;
  status: null | VecDbStatus;
  ragStatus: null | RagStatus;
};

const initialState: KnowledgeState = {
  loaded: false,
  memories: {},
  status: null,
  ragStatus: null,
};

export const knowledgeSlice = createSlice({
  name: "knowledge",
  initialState,
  reducers: {
    // TODO: add reducers
    setVecDbStatus: (state, action: PayloadAction<VecDbStatus>) => {
      state.loaded = true;
      state.status = action.payload;
    },
    setRagStatus: (state, action: PayloadAction<RagStatus>) => {
      state.loaded = true;
      state.ragStatus = action.payload;
      state.status = action.payload.vecdb;
    },
    setMemory: (state, action: PayloadAction<MemoRecord>) => {
      state.loaded = true;
      state.memories[action.payload.memid] = action.payload;
    },
    deleteMemory: (state, action: PayloadAction<string>) => {
      state.loaded = true;
      const { [action.payload]: _, ...memories } = state.memories;
      state.memories = memories;
    },
    clearMemory: (state) => {
      state.loaded = true;
      state.memories = {};
    },
  },
  // TODO: selectors
  selectors: {
    selectVecDbStatus: (state) => state.status,
    selectRagStatus: (state) => state.ragStatus,
    selectCodeGraphStatus: (state): CodeGraphStatus | null =>
      state.ragStatus?.codegraph ?? null,
    selectMemories: (state) => state.memories,
    selectKnowledgeIsLoaded: (state) => state.loaded,
  },
});

export const { setVecDbStatus, setRagStatus, setMemory, deleteMemory, clearMemory } =
  knowledgeSlice.actions;

export const {
  selectVecDbStatus,
  selectRagStatus,
  selectCodeGraphStatus,
  selectMemories,
  selectKnowledgeIsLoaded,
} = knowledgeSlice.selectors;
