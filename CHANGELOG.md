# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-08-15

### Added

- Bulk monitor actions: select rows in the monitors table, then enable,
  disable, check now, or delete the selection in one step. Delete asks for
  confirmation and states the count. An unknown id fails the whole batch
  rather than half-applying it.
- Import and sync monitors from a JSON file: pick the file through the native
  dialog, preview the add, update, and delete-missing diff, and apply it in a
  single transaction only on confirm. Absent keys keep their current values,
  so a short hand-written file stays safe to apply.

### Fixed

- Certificate expiry is derived in one place and shared by the monitors list
  and the monitor detail page. A certificate whose expiry has already passed
  now reads as "Expired" on both screens, instead of an amber "Expires in 0d"
  on one and a red negative day count on the other.

### Documentation

- README shows the dashboard and monitors screenshots.

## [0.1.0] - 2026-08-12

First release, published under the Clockwerk name.

### Added

- SQLite data layer covering monitor CRUD and application settings.
- Background scheduler, HTTP checker, and up/down state machine.
- Monitor management screens with routing and a settings page.
- Dashboard statistics and per-monitor history.
- TLS certificate monitoring for validity and upcoming expiry.
- Gap detection in history, and bounded retention of stored checks.
- Uptime alerts delivered as native notifications and to Slack, with Slack
  webhook configuration in settings.
- Continuous integration running the frontend and Rust checks on pull
  requests.

### Changed

- The application is named Clockwerk: the mark, app icons, and tray glyph ship
  with it, along with the Clockwerk palette, typography, and an About dialog.

### Fixed

- Slack webhooks can be verified and removed.
- Incident delivery is serialized with monitor state changes, so an alert
  cannot be sent for a state the store never committed.
- Delivery and settings contracts are preserved across restarts.

[0.2.0]: https://github.com/rcscatapang/clockwerk-uptime/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rcscatapang/clockwerk-uptime/releases/tag/v0.1.0
