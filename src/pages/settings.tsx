import { useState, type FormEvent } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  useSetSlackWebhook,
  useSettings,
  useUpdateSettings,
} from "@/lib/queries";

export function SettingsPage() {
  const [webhookUrl, setWebhookUrl] = useState("");
  const settings = useSettings();
  const updateSettings = useUpdateSettings();
  const setSlackWebhook = useSetSlackWebhook({
    onSuccess: () => setWebhookUrl(""),
  });

  function saveWebhook(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSlackWebhook.mutate(webhookUrl.trim());
  }

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
                updateSettings.mutate({
                  autostartEnabled: checked,
                  slackWebhookConfigured:
                    settings.data?.slackWebhookConfigured ?? false,
                  historyRetentionDays:
                    settings.data?.historyRetentionDays ?? 90,
                  lastPruneAt: settings.data?.lastPruneAt ?? null,
                })
              }
            />
          </div>
          <div className="mt-4 border-t pt-4">
            <p className="text-sm font-medium">
              History retention: {settings.data?.historyRetentionDays ?? 90} days
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              Last pruned: {settings.data?.lastPruneAt
                ? new Date(settings.data.lastPruneAt).toLocaleString()
                : "Not yet pruned"}
            </p>
          </div>
          <div className="mt-4 border-t pt-4">
            <p className="text-sm font-medium">Native notifications</p>
            <p className="mt-1 text-xs text-muted-foreground">
              macOS asks for permission when the first alert is sent. If alerts
              do not appear, enable notifications for this app in System
              Settings.
            </p>
          </div>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            Slack alerts
            {!settings.isPending && !settings.isError && (
              <Badge variant={settings.data?.slackWebhookConfigured ? "default" : "secondary"}>
                {settings.data?.slackWebhookConfigured ? "Configured" : "Not configured"}
              </Badge>
            )}
          </CardTitle>
          <CardDescription>
            Alert when a monitor goes down, remains down for an hour, or
            recovers. The webhook is stored in macOS Keychain and is never
            shown again.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <form className="space-y-3" onSubmit={saveWebhook}>
            <div className="space-y-2">
              <Label htmlFor="slack-webhook">Slack webhook URL</Label>
              <Input
                id="slack-webhook"
                type="password"
                autoComplete="off"
                value={webhookUrl}
                placeholder="https://hooks.slack.com/services/…"
                disabled={setSlackWebhook.isPending}
                onChange={(event) => setWebhookUrl(event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Saving sends a test message before replacing the Keychain value.
              </p>
            </div>
            <div className="flex gap-2">
              <Button
                type="submit"
                disabled={setSlackWebhook.isPending || webhookUrl.trim() === ""}
              >
                {setSlackWebhook.isPending ? "Saving…" : "Save webhook"}
              </Button>
              {settings.data?.slackWebhookConfigured && (
                <Button
                  type="button"
                  variant="outline"
                  disabled={setSlackWebhook.isPending}
                  onClick={() => setSlackWebhook.mutate("")}
                >
                  Remove webhook
                </Button>
              )}
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
