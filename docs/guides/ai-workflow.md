# Guide: AI Usage & Prompting Protocol

This guide outlines how our team uses AI tools (like LLMs, Copilot) to accelerate development without compromising code quality or security.

## 1. The Core Philosophy
* **AI is an Assistant, Not an Architect:** AI is excellent at writing boilerplate, generating tests, and formatting documentation. However, humans own the architecture. Never copy-paste complex Rust lifetimes or Kubernetes NetworkPolicies without fully understanding them.
* **Trust, but Verify (TDD):** If an AI generates a function, it must pass our automated tests. If it generates tests, a human must verify those tests actually cover the edge cases.

## 2. Prompting Rules for Hermes
To get the best results from an AI, you must provide it with our "Context".

* **Always Feed the ADRs:** If you ask an AI to design a feature, paste our `001-tech-stack.md` and the relevant DB Spec first. 
  * *Example Prompt:* "Act as a Senior Rust Developer. Based on this DB Schema for 'projects' [paste schema], write the SQLx insert query and the Axum handler. Use standard error handling."
* **Use Templates:** When generating documentation, give the AI the empty template and your rough notes.
  * *Example Prompt:* "Take these rough notes about the Login Page and format them strictly according to this UI Spec Template [paste template]."

## 3. Frontend (Angular) Specifics
* Ask the AI to use **Angular 17+ features** (Signals, new control flow `@if`, `@for`). Many AIs default to older RxJS patterns if not explicitly instructed.
* Always ask for UI component tests alongside the component code.

## 4. Security & Privacy (CRITICAL)
* **Never** paste real production `.env` variables, JWT secrets, DB passwords, or real user data into any AI prompt.
* **Never** paste SSL private keys or Kubernetes secret manifests.
* Use dummy data (e.g., `secret_key_123`) when asking for debugging help.