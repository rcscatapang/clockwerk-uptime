# Frontend conventions

- **Typed client only.** Components never call Tauri's `invoke` directly.
  Every command has a typed function in `src/lib/tauri.ts`, and components
  reach those through the TanStack Query hooks in `src/lib/queries.ts`.
- **Query keys.** `["monitors"]` for the list, `["monitors", id]` for a
  detail, `["settings"]` for settings. Mutations invalidate the keys they
  affect; no optimistic updates.
- **Errors.** Commands reject with `{ code, message }` (see the contract in
  `src/lib/tauri.ts`). Mutations surface failures as a toast (`sonner`) by
  default; forms that can map a code onto a field (e.g. `DuplicateUrl` → the
  URL input) handle `onError` themselves and fall back to the toast.
- **Routing.** `react-router` with three routes: `/` (dashboard),
  `/monitors`, `/settings`. Navigation lives in `components/app-layout.tsx`.
- **UI primitives** come from shadcn/ui (`components/ui/`); don't hand-roll
  buttons, dialogs, or form controls.
