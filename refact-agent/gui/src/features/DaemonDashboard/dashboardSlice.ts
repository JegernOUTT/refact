import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { RootState } from "../../app/store";
import type { DaemonEvent } from "../../services/refact/daemon";

export type DashboardPage =
  | "home"
  | "projects"
  | "activity"
  | "scheduler"
  | "usage"
  | "doctor"
  | "settings";

export type DashboardNavigation = {
  page: DashboardPage;
  params: Record<string, string>;
};

export type DashboardStreamStatus = "connecting" | "connected" | "reconnecting";

export type DashboardState = {
  navigation: DashboardNavigation;
  events: DaemonEvent[];
  streamStatus: DashboardStreamStatus;
};

const MAX_DAEMON_EVENTS = 500;

const initialState: DashboardState = {
  navigation: { page: "home", params: {} },
  events: [],
  streamStatus: "connecting",
};

export const dashboardSlice = createSlice({
  name: "daemonDashboard",
  initialState,
  reducers: {
    navigateDashboard: (state, action: PayloadAction<DashboardNavigation>) => {
      state.navigation = action.payload;
    },
    daemonEventsReceived: (state, action: PayloadAction<DaemonEvent[]>) => {
      const eventsBySequence = new Map(
        state.events.map((event) => [event.seq, event]),
      );
      for (const event of action.payload) {
        eventsBySequence.set(event.seq, event);
      }
      state.events = [...eventsBySequence.values()]
        .sort((left, right) => left.seq - right.seq)
        .slice(-MAX_DAEMON_EVENTS);
    },
    daemonEventsReset: (state) => {
      state.events = [];
    },
    daemonStreamStatusChanged: (
      state,
      action: PayloadAction<DashboardStreamStatus>,
    ) => {
      state.streamStatus = action.payload;
    },
  },
});

export const {
  daemonEventsReceived,
  daemonEventsReset,
  daemonStreamStatusChanged,
  navigateDashboard,
} = dashboardSlice.actions;

export const selectDashboardNavigation = (state: RootState) =>
  state.daemonDashboard.navigation;
export const selectDashboardPage = (state: RootState) =>
  state.daemonDashboard.navigation.page;
export const selectDashboardParams = (state: RootState) =>
  state.daemonDashboard.navigation.params;
export const selectDaemonEvents = (state: RootState) =>
  state.daemonDashboard.events;
export const selectDaemonStreamStatus = (state: RootState) =>
  state.daemonDashboard.streamStatus;
