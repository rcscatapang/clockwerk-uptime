// TanStack Query hooks over the typed Tauri client.
//
// Conventions (see src/README.md):
// - Components never call the client (or `invoke`) directly — they use these
//   hooks, so query keys and invalidation stay in one place.
// - Query keys: ["monitors"], ["monitors", id], ["settings"].
// - Mutations invalidate the affected keys and surface failures as a toast;
//   forms that want inline errors handle them in their own onError first.

import { useEffect } from "react";

import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";

import { errorMessage } from "@/lib/errors";
import * as api from "@/lib/tauri";
import type {
  CheckSummary,
  HistoryRange,
  Monitor,
  MonitorInput,
  Settings,
  SyncResult,
} from "@/lib/tauri";

export const monitorKeys = {
  all: ["monitors"] as const,
  detail: (id: number) => ["monitors", id] as const,
};

export const statsKeys = {
  all: ["stats"] as const,
  detail: (id: number) => ["stats", id] as const,
};

export const historyKeys = {
  all: ["history"] as const,
  detail: (id: number, range?: HistoryRange) =>
    range === undefined
      ? (["history", id] as const)
      : (["history", id, range] as const),
};

export const settingsKeys = {
  all: ["settings"] as const,
};

export const syncKeys = {
  preview: (path: string | null, deleteMissing: boolean) =>
    ["monitor-sync-preview", path, deleteMissing] as const,
};

export function useMonitors() {
  return useQuery({ queryKey: monitorKeys.all, queryFn: api.listMonitors });
}

export function useMonitor(id: number) {
  return useQuery({
    queryKey: monitorKeys.detail(id),
    queryFn: () => api.getMonitor(id),
  });
}

export function useUptimeStats(id: number) {
  return useQuery({
    queryKey: statsKeys.detail(id),
    queryFn: () => api.getUptimeStats(id),
  });
}

export function useHistory(id: number, range: HistoryRange) {
  return useQuery({
    queryKey: historyKeys.detail(id, range),
    queryFn: () => api.getHistory(id, range),
  });
}

export function useSettings() {
  return useQuery({ queryKey: settingsKeys.all, queryFn: api.getSettings });
}

type MutationOpts<TData, TVariables> = Pick<
  UseMutationOptions<TData, unknown, TVariables>,
  "onSuccess" | "onError"
>;

function useInvalidatingMutation<TData, TVariables>(
  mutationFn: (vars: TVariables) => Promise<TData>,
  keys: readonly (readonly unknown[])[],
  opts?: MutationOpts<TData, TVariables>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: (data, variables, onMutateResult, context) => {
      for (const key of keys) {
        queryClient.invalidateQueries({ queryKey: key });
      }
      return opts?.onSuccess?.(data, variables, onMutateResult, context);
    },
    onError: (error, variables, onMutateResult, context) => {
      if (opts?.onError) {
        return opts.onError(error, variables, onMutateResult, context);
      }
      toast.error(errorMessage(error));
    },
  });
}

export function useCreateMonitor(opts?: MutationOpts<Monitor, MonitorInput>) {
  return useInvalidatingMutation(api.createMonitor, [monitorKeys.all], opts);
}

export function useUpdateMonitor(
  opts?: MutationOpts<Monitor, { id: number; input: MonitorInput }>,
) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id: number; input: MonitorInput }) =>
      api.updateMonitor(id, input),
    onSuccess: (data, variables, onMutateResult, context) => {
      queryClient.invalidateQueries({ queryKey: monitorKeys.all });
      queryClient.invalidateQueries({ queryKey: monitorKeys.detail(data.id) });
      queryClient.invalidateQueries({ queryKey: statsKeys.detail(data.id) });
      queryClient.invalidateQueries({ queryKey: historyKeys.detail(data.id) });
      return opts?.onSuccess?.(data, variables, onMutateResult, context);
    },
    onError: (error, variables, onMutateResult, context) => {
      if (opts?.onError) {
        return opts.onError(error, variables, onMutateResult, context);
      }
      toast.error(errorMessage(error));
    },
  });
}

export function useDeleteMonitor(opts?: MutationOpts<void, number>) {
  return useInvalidatingMutation(api.deleteMonitor, [
    monitorKeys.all,
    statsKeys.all,
    historyKeys.all,
  ], opts);
}

/** Bulk enable/disable. Touches `uptimeCheckEnabled` only. */
export function useSetMonitorsEnabled(
  opts?: MutationOpts<number, { ids: number[]; enabled: boolean }>,
) {
  return useInvalidatingMutation(
    ({ ids, enabled }: { ids: number[]; enabled: boolean }) =>
      api.setMonitorsEnabled(ids, enabled),
    [monitorKeys.all],
    opts,
  );
}

export function useDeleteMonitors(opts?: MutationOpts<number, number[]>) {
  return useInvalidatingMutation(
    api.deleteMonitors,
    [monitorKeys.all, statsKeys.all, historyKeys.all],
    opts,
  );
}

/**
 * Force a check of the given monitors. The backend also emits
 * `check-completed`, so rows refresh through the usual event path too.
 */
export function useCheckMonitors(opts?: MutationOpts<CheckSummary, number[]>) {
  return useInvalidatingMutation(
    api.checkMonitors,
    [monitorKeys.all, statsKeys.all, historyKeys.all],
    opts,
  );
}

export function useCheckNow(opts?: MutationOpts<Monitor, number>) {
  return useInvalidatingMutation(api.checkNow, [
    monitorKeys.all,
    statsKeys.all,
    historyKeys.all,
  ], opts);
}

/**
 * Preview a sync file. The backend writes nothing here, so this is a plain
 * query: the file and the delete-missing toggle are its only inputs, and the
 * dialog shows its error itself rather than as a toast. Never cached — the
 * file can change on disk between openings.
 */
export function useMonitorSyncPreview(
  path: string | null,
  deleteMissing: boolean,
) {
  return useQuery({
    queryKey: syncKeys.preview(path, deleteMissing),
    queryFn: () => api.previewMonitorSync(path as string, deleteMissing),
    enabled: path !== null,
    gcTime: 0,
    staleTime: 0,
    retry: false,
  });
}

export function useApplyMonitorSync(
  opts?: MutationOpts<SyncResult, { path: string; deleteMissing: boolean }>,
) {
  return useInvalidatingMutation(
    ({ path, deleteMissing }: { path: string; deleteMissing: boolean }) =>
      api.applyMonitorSync(path, deleteMissing),
    [monitorKeys.all, statsKeys.all, historyKeys.all],
    opts,
  );
}

export function useUpdateSettings(opts?: MutationOpts<Settings, Settings>) {
  return useInvalidatingMutation(api.updateSettings, [settingsKeys.all], opts);
}

export function useSetSlackWebhook(
  opts?: MutationOpts<Settings, string>,
) {
  return useInvalidatingMutation(api.setSlackWebhook, [settingsKeys.all], opts);
}

/**
 * Refresh monitor data whenever the Rust engine finishes a check cycle.
 * This is the only push channel from the backend — no polling anywhere.
 */
export function useCheckCompletedInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const unlisten = listen<{ monitorIds: number[] }>("check-completed", (event) => {
      queryClient.invalidateQueries({ queryKey: monitorKeys.all });
      for (const id of event.payload.monitorIds) {
        queryClient.invalidateQueries({ queryKey: monitorKeys.detail(id) });
        queryClient.invalidateQueries({ queryKey: statsKeys.detail(id) });
        queryClient.invalidateQueries({ queryKey: historyKeys.detail(id) });
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [queryClient]);
}
