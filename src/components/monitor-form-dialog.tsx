import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/errors";
import {
  emptyFormValues,
  fieldForErrorCode,
  formValuesFromMonitor,
  toMonitorInput,
  urlScheme,
  validateForm,
  type MonitorFormErrors,
  type MonitorFormValues,
} from "@/lib/monitor-form";
import { useCreateMonitor, useUpdateMonitor } from "@/lib/queries";
import { isAppError, type CheckMethod, type Monitor } from "@/lib/tauri";

interface MonitorFormDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Present when editing; absent when creating. */
  monitor?: Monitor;
}

export function MonitorFormDialog({
  open,
  onOpenChange,
  monitor,
}: MonitorFormDialogProps) {
  const [values, setValues] = useState<MonitorFormValues>(emptyFormValues);
  const [errors, setErrors] = useState<MonitorFormErrors>({});
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setValues(monitor ? formValuesFromMonitor(monitor) : emptyFormValues());
      setErrors({});
      setFormError(null);
    }
  }, [open, monitor]);

  const onServerError = (e: unknown) => {
    if (isAppError(e)) {
      const field = fieldForErrorCode(e.code);
      if (field) {
        setErrors((prev) => ({ ...prev, [field]: errorMessage(e) }));
        return;
      }
      setFormError(errorMessage(e));
      return;
    }
    toast.error(errorMessage(e));
  };

  const createMonitor = useCreateMonitor({
    onSuccess: () => onOpenChange(false),
    onError: onServerError,
  });
  const updateMonitor = useUpdateMonitor({
    onSuccess: () => onOpenChange(false),
    onError: onServerError,
  });
  const isPending = createMonitor.isPending || updateMonitor.isPending;

  const scheme = urlScheme(values.url);
  const isHead = values.checkMethod === "HEAD";

  const setField = <K extends keyof MonitorFormValues>(
    field: K,
    value: MonitorFormValues[K],
  ) => {
    setValues((prev) => {
      const next = { ...prev, [field]: value };
      // Follow the backend's scheme rule without clobbering an explicit
      // choice: adjust the cert toggle only when the URL's scheme changes.
      if (field === "url" && urlScheme(next.url) !== urlScheme(prev.url)) {
        next.certCheckEnabled = urlScheme(next.url) === "https";
      }
      // HEAD has no body to search; drop any leftover text so the disabled
      // input and the submitted value agree.
      if (field === "checkMethod" && value === "HEAD") {
        next.lookForString = "";
      }
      return next;
    });
    setErrors((prev) => ({ ...prev, [field]: undefined }));
    setFormError(null);
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const validation = validateForm(values);
    setErrors(validation);
    if (Object.values(validation).some(Boolean)) return;
    const input = toMonitorInput(values);
    if (monitor) {
      updateMonitor.mutate({ id: monitor.id, input });
    } else {
      createMonitor.mutate(input);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{monitor ? "Edit monitor" : "Add monitor"}</DialogTitle>
          <DialogDescription>
            {monitor
              ? "Change how this URL is checked."
              : "Add a URL to check on a schedule."}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} noValidate className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="monitor-url">URL</Label>
            <Input
              id="monitor-url"
              placeholder="https://example.com"
              value={values.url}
              onChange={(e) => setField("url", e.target.value)}
              autoFocus
            />
            {errors.url && (
              <p className="text-sm text-destructive">{errors.url}</p>
            )}
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="monitor-interval">Interval (minutes)</Label>
              <Input
                id="monitor-interval"
                type="number"
                min={1}
                value={values.checkIntervalMinutes}
                onChange={(e) =>
                  setField("checkIntervalMinutes", e.target.value)
                }
              />
              {errors.checkIntervalMinutes && (
                <p className="text-sm text-destructive">
                  {errors.checkIntervalMinutes}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label htmlFor="monitor-method">Method</Label>
              <Select
                value={values.checkMethod}
                onValueChange={(v) => setField("checkMethod", v as CheckMethod)}
              >
                <SelectTrigger id="monitor-method" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="GET">GET</SelectItem>
                  <SelectItem value="HEAD">HEAD</SelectItem>
                  <SelectItem value="POST">POST</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="monitor-look-for">Look for string</Label>
            <Input
              id="monitor-look-for"
              placeholder="Optional text the response body must contain"
              value={values.lookForString}
              disabled={isHead}
              onChange={(e) => setField("lookForString", e.target.value)}
            />
            {isHead ? (
              <p className="text-sm text-muted-foreground">
                Not available for HEAD — there is no body to search.
              </p>
            ) : (
              errors.lookForString && (
                <p className="text-sm text-destructive">
                  {errors.lookForString}
                </p>
              )
            )}
          </div>

          <div className="flex items-center justify-between">
            <Label htmlFor="monitor-uptime-enabled">Uptime checks</Label>
            <Switch
              id="monitor-uptime-enabled"
              checked={values.uptimeCheckEnabled}
              onCheckedChange={(v) => setField("uptimeCheckEnabled", v)}
            />
          </div>

          <div className="flex items-center justify-between">
            <div>
              <Label htmlFor="monitor-cert-enabled">Certificate checks</Label>
              {scheme !== "https" && (
                <p className="text-sm text-muted-foreground">
                  Requires an https URL.
                </p>
              )}
            </div>
            <Switch
              id="monitor-cert-enabled"
              checked={values.certCheckEnabled}
              disabled={scheme !== "https"}
              onCheckedChange={(v) => setField("certCheckEnabled", v)}
            />
          </div>

          {formError && <p className="text-sm text-destructive">{formError}</p>}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isPending}>
              {monitor ? "Save changes" : "Add monitor"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
