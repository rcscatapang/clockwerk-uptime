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
import type { Monitor, MonitorInput, Settings } from "@/lib/tauri";

export const monitorKeys = {
  all: ["monitors"] as const,
};

export const settingsKeys = {
  all: ["settings"] as const,
};

export function useMonitors() {
  return useQuery({ queryKey: monitorKeys.all, queryFn: api.listMonitors });
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
  return useInvalidatingMutation(
    ({ id, input }) => api.updateMonitor(id, input),
    [monitorKeys.all],
    opts,
  );
}

export function useDeleteMonitor(opts?: MutationOpts<void, number>) {
  return useInvalidatingMutation(api.deleteMonitor, [monitorKeys.all], opts);
}

export function useCheckNow(opts?: MutationOpts<Monitor, number>) {
  return useInvalidatingMutation(api.checkNow, [monitorKeys.all], opts);
}

export function useUpdateSettings(opts?: MutationOpts<Settings, Settings>) {
  return useInvalidatingMutation(api.updateSettings, [settingsKeys.all], opts);
}

/**
 * Refresh monitor data whenever the Rust engine finishes a check cycle.
 * This is the only push channel from the backend — no polling anywhere.
 */
export function useCheckCompletedInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const unlisten = listen("check-completed", () => {
      queryClient.invalidateQueries({ queryKey: monitorKeys.all });
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [queryClient]);
}
