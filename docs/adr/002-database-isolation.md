# ADR 002: Database Isolation Strategy

**Date:** [15-04-2026]
**Status:** Accepted

## Context
Hermes provides managed databases (PostgreSQL, MongoDB, Redis) to its users. We need to decide how these databases are provisioned and isolated within our Kubernetes-based infrastructure. The main trade-off is between resource efficiency (shared instances) and operational security/isolation (dedicated instances).

## Decision
We will use a **Dedicated Instance** strategy. Every project will have its own independent database container (running as a Kubernetes `StatefulSet` with a `PersistentVolumeClaim`) within its dedicated Namespace.

## Rationale (Why?)
* **Strict Isolation:** This follows our core architecture where one Project equals one Kubernetes Namespace. It prevents "Noisy Neighbor" effects where one user's heavy queries could impact another user's performance.
* **Security:** Data is physically separated at the container and volume level. A security breach in one database instance does not grant access to others.
* **Resource Control:** We can use Kubernetes `ResourceQuotas` to limit CPU and RAM usage per database instance, ensuring predictable billing and cluster stability.
* **Customization:** Users can eventually choose specific database versions or configurations without affecting the rest of the platform.

## Rejected Alternatives
* **Shared Instance (Multi-tenancy via Logical DBs):** While more resource-efficient (lower RAM usage), it was rejected because it introduces a single point of failure and makes performance isolation nearly impossible for an MVP that aims for enterprise-grade reliability.

## Consequences
* **Positive:** Highest possible isolation and security. Easier to track and bill resource usage per project.
* **Negative:** Higher resource overhead (RAM/CPU) per project, as even an empty database requires its own container overhead. This increases the minimum system requirements for running Hermes.