import { HashRouter, Route, Routes } from "react-router";

import { AppLayout } from "@/components/app-layout";
import { Toaster } from "@/components/ui/sonner";
import { useCheckCompletedInvalidation } from "@/lib/queries";
import { DashboardPage } from "@/pages/dashboard";
import { MonitorsPage } from "@/pages/monitors";
import { SettingsPage } from "@/pages/settings";

export default function App() {
  useCheckCompletedInvalidation();
  return (
    <HashRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route path="/" element={<DashboardPage />} />
          <Route path="/monitors" element={<MonitorsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Route>
      </Routes>
      <Toaster position="bottom-right" />
    </HashRouter>
  );
}
