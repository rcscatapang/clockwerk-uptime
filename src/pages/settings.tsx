import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useSettings, useUpdateSettings } from "@/lib/queries";

export function SettingsPage() {
  const settings = useSettings();
  const updateSettings = useUpdateSettings();

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight">Settings</h1>
      <Card>
        <CardHeader>
          <CardTitle>General</CardTitle>
          <CardDescription>
            The app lives in the menubar; closing the window keeps monitoring
            running.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {settings.isError && (
            <p className="mb-4 text-sm text-destructive">
              Could not load settings. {String(settings.error)}
            </p>
          )}
          <div className="flex items-center justify-between">
            <Label htmlFor="autostart">Launch at login</Label>
            <Switch
              id="autostart"
              checked={settings.data?.autostartEnabled ?? false}
              disabled={settings.isPending || updateSettings.isPending}
              onCheckedChange={(checked) =>
                updateSettings.mutate({ autostartEnabled: checked })
              }
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
