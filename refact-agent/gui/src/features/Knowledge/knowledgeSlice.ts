import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import type {
  CodeGraphStatus,
  MemoRecord,
  RagStatus,
} from "../../services/refact/types";

export type KnowledgeState = {
  loaded: boolean;
  memories: Record<string, MemoRecord>;
  ragStatus: null | RagStatus;
};

const initialState: KnowledgeState = {
  loaded: false,
  memories: {},
  ragStatus: null,
};

export const knowledgeSlice = createSlice({
  name: "knowledge",
  initialState,
  reducers: {
    // TODO: add reducers
    setRagStatus: (state, action: PayloadAction<RagStatus>) => {
      state.loaded = true;
      state.ragStatus = action.payload;
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
    selectRagStatus: (state) => state.ragStatus,
    selectCodeGraphStatus: (state): CodeGraphStatus | null =>
      state.ragStatus?.codegraph ?? null,
    selectMemories: (state) => state.memories,
    selectKnowledgeIsLoaded: (state) => state.loaded,
  },
});

export const { setRagStatus, setMemory, deleteMemory, clearMemory } =
  knowledgeSlice.actions;

export const {
  selectRagStatus,
  selectCodeGraphStatus,
  selectMemories,
  selectKnowledgeIsLoaded,
} = knowledgeSlice.selectors;
