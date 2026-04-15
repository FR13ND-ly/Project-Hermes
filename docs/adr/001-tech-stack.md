# ADR 001: Core Technology Stack

**Date:** 2024-04-15
**Status:** Accepted

## Context
We are building Hermes, a Platform-as-a-Service (PaaS) and Backend-as-a-Service (BaaS) designed to orchestrate containers, databases, and network routing. The system requires high performance, absolute memory safety, and a predictable user interface. As a small team of two developers (one backend, one frontend), we need a technology stack that minimizes runtime errors, enforces strict architectural patterns, and allows for rapid but safe iterations.

## Decision
We will build the Hermes platform using the following core technologies:
* **Backend:** Rust with the Axum framework.
* **Frontend:** Angular (v21+) using Signals.
* **Database:** PostgreSQL (interfaced via SQLx in Rust).
* **Infrastructure / Orchestration:** K3s (Lightweight Kubernetes).
* **Gateway / Routing:** Nginx (Hardcoded configuration managed by Rust).

## Rationale (Why?)
* **Rust & Axum:** Rust provides C-level performance with mathematical guarantees against memory leaks and data races. Axum is built on the Tokio ecosystem, providing highly concurrent, type-safe HTTP routing.
* **Angular:** As an enterprise-grade framework, Angular provides a highly opinionated structure (Dependency Injection, Routing, RxJS/Signals). This eliminates "JavaScript fatigue" and ensures the frontend architecture remains clean as the dashboard scales.
* **PostgreSQL & SQLx:** We are building a "hardcoded" MVP with strict relational data (Users -> Projects -> Deployments). Postgres is the industry standard for data integrity. Using SQLx allows us to verify SQL queries at compile time against a live database.
* **K3s:** It provides 100% of the Kubernetes API but is lightweight enough to run efficiently on single-node servers or local development environments.
* **Nginx:** Instead of using complex Kubernetes Ingress controllers (like Traefik or Nginx-Ingress) which require CRDs and heavy K8s knowledge, we use a single, hardcoded Nginx instance. The Rust backend will dynamically generate `nginx.conf` blocks and trigger a reload (`nginx -s reload`) whenever a user adds a new Custom Domain. This "KISS" (Keep It Simple, Stupid) approach guarantees we understand 100% of the routing logic for our MVP.

## Rejected Alternatives
* **Backend - Node.js / Go:** Node.js was rejected due to higher memory footprint and lack of strict runtime type safety. Go was a strong contender for cloud infrastructure, but Rust's expressive type system and pattern matching provide better safety for complex business logic.
* **Frontend - React / Vue:** React was rejected because it is too unopinionated, requiring the team to make too many decisions regarding third-party routing and state management libraries.
* **Database - MongoDB:** Rejected because user roles and infrastructure metadata require strict relational schemas, not document flexibility.

## Consequences
* **Positive:** Compile-time safety across the entire backend. Predictable and scalable UI architecture. Strict data integrity.
* **Negative:** Rust has a steep learning curve and longer compile times, which may slow down initial backend development compared to interpreted languages.