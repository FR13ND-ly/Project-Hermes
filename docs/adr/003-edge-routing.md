# ADR 003: Edge Routing and Gateway Management

**Date:** 15-04-2026
**Status:** Accepted

## Context
Hermes must route external HTTP/HTTPS traffic from public domains (both system-provided and user-custom) to specific internal Kubernetes workloads (Pods/Deployments). The system must handle SSL certificate generation, support custom proxy rules for power users, and provide a transparent way to debug routing issues.

## Decision
We will use a standalone **Nginx Gateway** managed directly by the Rust backend acting as the Control Plane, bypassing native Kubernetes Ingress Controllers.

Furthermore, we adopt a **State Reconciliation Pattern**: The Rust backend will generate the Nginx configuration blocks dynamically, apply them, and save the exact generated string in the database (`last_applied_config`).

## Rationale (Why?)
* **Simplicity (KISS):** Native K8s Ingress Controllers (like Traefik or Nginx-Ingress) require complex Custom Resource Definitions (CRDs) and heavy Kubernetes API interactions. By having Rust write raw `nginx.conf` files and triggering an `nginx -s reload`, we eliminate a massive layer of operational complexity.
* **Transparency:** Storing the `last_applied_config` allows the Angular dashboard to show users exactly what Nginx block is routing their traffic. This makes debugging incredibly easy and builds trust.
* **Customization:** It allows us to easily support a `custom` domain type where power users can write their own raw Nginx directives (e.g., custom caching headers, specific WebSocket timeouts) without fighting the limitations of Kubernetes Ingress annotations.

## Rejected Alternatives
* **Native Kubernetes Ingress:** Rejected due to complexity. It abstracts away the routing layer too much, making it very difficult to show the user the "actual" routing configuration in the Hermes dashboard.
* **Caddy Server:** While Caddy has automatic HTTPS and an API, Nginx remains the industry standard. Most developers know how to write an Nginx block, which is critical for our `custom` domain routing feature.

## Consequences
* **Positive:** Complete control over the routing logic. The UI can display real-time, accurate routing configs. Easy to implement custom SSL (Certbot) workers in Rust.
* **Negative:** Triggering `nginx -s reload` for every new domain works perfectly for hundreds or thousands of domains, but may introduce slight latency spikes if scaling to tens of thousands of domains simultaneously.