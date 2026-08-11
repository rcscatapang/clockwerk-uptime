import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AppLayout } from "@/components/app-layout";
import { Button } from "@/components/ui/button";
import { getSettings, updateSettings } from "@/lib/tauri";

// Placeholder screen for issue 01: proves Tailwind + shadcn/ui render and that
// the frontend can round-trip a Tauri command (autostart toggle).
export default function App() {
  const queryClient = useQueryClient();

  const settings = useQuery({
    queryKey: ["settings"],
    queryFn: getSettings,
  });

  const toggleAutostart = useMutation({
    mutationFn: updateSettings,
    onSuccess: (next) => {
      queryClient.setQueryData(["settings"], next);
    },
  });

  const autostart = settings.data?.autostart_enabled;

  return (
    <AppLayout>
      <div className="mx-auto flex max-w-md flex-col items-center gap-4 pt-16 text-center">
        <h1 className="text-2xl font-bold tracking-tight">
          Scaffold is running
        </h1>
        <p className="text-sm text-muted-foreground">
          Tauri 2 · React 19 · Tailwind v4 · shadcn/ui · TanStack Query.
          Close this window and the app keeps running in the menubar.
        </p>
        <Button
          disabled={settings.isPending || toggleAutostart.isPending}
          onClick={() =>
            toggleAutostart.mutate({ autostart_enabled: !autostart })
          }
        >
          {settings.isPending
            ? "Loading…"
            : `Launch at login: ${autostart ? "on" : "off"}`}
        </Button>
        {(settings.isError || toggleAutostart.isError) && (
          <p className="text-sm text-destructive">
            {String(settings.error ?? toggleAutostart.error)}
          </p>
        )}
      </div>
    </AppLayout>
  );
}
