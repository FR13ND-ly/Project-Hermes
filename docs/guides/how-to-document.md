# How to Write Documentation for Hermes

Welcome to the Hermes documentation guide. We treat our documentation as code (Docs-as-Code). To maintain a single source of truth and avoid ambiguities, please follow these rules:

## 1. Golden Rules
* **English Only:** All architecture, API contracts, and UI specs must be written in English.
* **No Code Duplication:** Do not paste huge blocks of Rust or Angular code here. Document the *behavior*, *contracts*, and *architecture*.
* **Use Templates:** Never start a spec from a blank page. Copy the relevant template from the `docs/templates/` directory.

## 2. File Naming Conventions
To ensure sorting works correctly and links do not break, strictly follow these rules:
* **Format:** Use strictly lowercase `kebab-case` for all files and folders. No spaces, no CamelCase, no underscores.
  * ❌ `User Flow.md`, `Login_Page.md`, `apiSpecs.md`
  * ✅ `user-flow.md`, `login-page.md`, `api-specs.md`
* **Sequencing (UI Specs):** Prefix UI specs with a two-digit number based on the typical user journey.
  * ✅ `01-login.md`, `02-dashboard.md`, `03-project-overview.md`
* **Sequencing (ADRs):** Prefix Architecture Decision Records with a three-digit sequential number.
  * ✅ `001-tech-stack.md`, `002-methodology.md`

## 3. Formatting & Markdown Standards
* **Headings:** Use `#` (H1) strictly once per file for the main title. Use `##` (H2) for main sections and `###` (H3) for sub-sections.
* **Code Blocks:** Always specify the syntax highlighting language for code blocks (e.g., ````json`, ````rust`, ````typescript`).
* **Linking:** When referring to another document, use relative paths.
  * ✅ `As defined in the [Project API Spec](../api/projects-api.md)`

## 4. Where Things Belong
* **`/adr` (Architecture Decision Records):** Use this when making a major technical choice (e.g., choosing PostgreSQL over MongoDB).
* **`/specs/ui` (UI Specs):** Frontend behavior, user flows, and state management.
* **`/specs/api` (API Contracts):** JSON structure agreements between the Angular frontend and Rust backend.
* **`/specs/db` (Database):** SQL schemas and entity relationships.

## 5. Workflow
1. Copy a template from `docs/templates/`.
2. Paste it into the appropriate `docs/specs/` folder and rename it following the conventions.
3. Fill in the details.
4. Submit it as part of your Pull Request.