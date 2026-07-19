# Shennong UI design QA

Status: passed on the live deployment at `http://192.168.3.10:18081`.

## Coverage

- Roles: visitor, authenticated researcher, administrator.
- Viewports: 1280 x 720 desktop and 390 x 844 mobile.
- Journeys: public chat and Docs; sign-in; personal and Project chat; search;
  profile and settings; Resources and fixed detail drawer; Projects list and
  workspace; Plugins and Skills; upload-to-Agent handoff; Compute; admin
  dashboard, users, providers, invitations, audit pagination, registration,
  and backups.

## Product and taste judgment

- The product now uses one shell language: restrained scientific teal actions,
  consistent neutral surfaces, shared sidebar geometry, matching typography,
  and predictable active, empty, loading, error, dialog, and drawer states.
- Visitors can understand the Agent value before signing in, but protected
  runtime and composer actions remain unavailable.
- Research work can begin without a Project; Project selection is reserved for
  governed data, Runtime, and artifact workflows.
- The administrator experience keeps the same navigation and interaction
  grammar while using denser production tables and operational status panels.
- Mobile navigation no longer obscures long-page content after scrolling;
  wide operational tables remain reachable inside horizontal scroll regions
  without widening the document.

## Verified evidence

- Visitor: `output/playwright/shennong-ui-qa/visitor/`
- Researcher desktop/mobile: `output/playwright/shennong-ui-qa/user/` and
  `output/playwright/shennong-ui-qa/mobile/`
- Administrator desktop/mobile recheck:
  `output/playwright/shennong-ui-qa/admin-reverify/`
- Chat failure regression: canonical UUID thread, visible
  `credential_unavailable` alert, and durable Run status `failed` with no idle
  Stop control or draft-time message/history 404s.
- Upload-to-Agent regression: the clean Project chat URL received the optional
  background, uploaded filename, and governed `project://current/resources/...`
  reference; the one-time browser handoff was then removed.
- Search geometry: the 680 px desktop dialog is horizontally centered and its
  top edge sits at one-third of the 720 px viewport instead of touching the
  browser chrome.
- Browser console: zero errors and warnings in the final visitor, researcher,
  and administrator route checks.

Full-page screenshots can visually compress a fixed sidebar because of the
capture method. Viewport screenshots and measured browser geometry confirmed a
256 px desktop sidebar and correctly offset main content.
