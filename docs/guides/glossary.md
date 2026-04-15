# Glossary of Terms

To avoid confusion during development and code reviews, we use these terms with the following specific meanings:

| Term | Definition |
| :--- | :--- |
| **User** | A developer or admin who has an account on the Hermes platform. |
| **Project** | A logical container for resources, mapped 1-to-1 to a Kubernetes Namespace. |
| **Deployment** | A specific instance of a service (Frontend or Backend) running within a Project. |
| **Orchestrator** | The Rust/Axum backend logic that communicates with the K8s API. |
| **Hardcoded** | Refers to our MVP strategy of using strict, predefined logic instead of a generic plugin system. |
| **Platform Admin** | A User with `is_superadmin = true` who can see all projects and cluster health. |
| **Provisioning** | The state where Hermes is actively creating K8s resources or DB instances. |
| **BaaS** | Backend-as-a-Service (e.g., Auth-as-a-Service, Managed DBs). |
| **Tenant** | Another word for a Project, emphasizing the isolation between different users' environments. |