# Frontend Architecture (Angular)

**Date:** [15-04-2026]
**Status:** Active

This document outlines the strict coding standards and folder structure for the Hermes Angular application. Code that does not follow these conventions will be rejected during Pull Request reviews.

## 1. Core + Pages Architecture
The application is strictly divided into two main directories: `core` (reusable logic and UI) and `pages` (routed views).

**Structure:**
```text
/src/app
  /core
    /components         # Dumb/Presentational components (buttons, modals)
    /services           # HTTP API calls and global State management
    /guards             # Route guards (e.g., AuthGuard)
    /models             # TypeScript Interfaces and Enums
  /pages
    /dashboard          # Smart component folder (logic + template)
    /projects           # Smart component folder
```

## 2. File Naming Conventions
We drop the legacy verbose suffixes (`.component.ts`, `.service.ts`). The file's purpose is dictated by its folder and name.

* **Components:** `[name].ts` (e.g., `project-card.ts`, `dashboard-page.ts`)
* **Templates:** `[name].html` (e.g., `dashboard-page.html`)
* **Styles:** `[name].scss`
* **Services:** Postfix with purpose, e.g., `[name]-api.ts` or `[name]-state.ts`
* **Guards:** `[name]-guard.ts` (e.g., `auth-guard.ts`)
* **Models:** `[name].ts` (e.g., `project.ts`)

## 3. Selectors
We avoid overly specific global prefixes.
* **Pages:** Routed components typically don't need selectors used in HTML, but if necessary, use `page-` (e.g., `<page-dashboard>`).
* **Core Components:** Use `ui-` or `app-` for reusable elements (e.g., `<ui-button>`, `<app-project-card>`).

## 4. Component Architecture (Smart vs. Dumb)
* **Pages (Smart):** Reside in `/pages`. They are tied to routes, inject services, manage Signals, and pass data down to core components.
* **Core Components (Dumb):** Reside in `/core/components`. They only receive data via `@Input()` and emit actions via `@Output()`. They must **never** inject HTTP services.

## 5. State Management & Signals
* **Synchronous UI State:** Use **Signals** (`signal()`, `computed()`) for local UI state.
* **Asynchronous Events:** Use **RxJS** (`Observables`) ONLY for HTTP requests (`HttpClient`). Convert them using `toSignal()` for template rendering.
* **Control Flow:** Strictly use Angular 17+ block syntax (`@if`, `@for`, `@switch`).