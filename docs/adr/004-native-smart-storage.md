# ADR 004: Native Smart Storage & Media Processing

**Date:** 15-04-2026
**Status:** Accepted

## Context
Hermes needs to provide BaaS users with a way to upload, store, and serve files. Initially, a standard S3-compatible object storage (like MinIO) was considered. However, modern applications require more than just raw file hosting; they need optimized media (resized images, modern formats like WebP) and smooth UI transitions (Blurhash). Using a generic S3 provider would require an additional proxy or serverless functions to process images, increasing latency, operational complexity, and costs.

## Decision
We will bypass external Object Storage solutions (MinIO/S3) and build a **Native Smart Storage Pipeline** directly in Rust. 

The Rust backend will intercept file uploads, perform synchronous or asynchronous media processing (auto-resizing, WebP conversion, Blurhash generation), and store the resulting optimized binaries directly on shared Kubernetes Persistent Volumes (PVCs) mounted to the API pods. Metadata and quotas will be tracked in PostgreSQL.

## Rationale (Why?)
* **Superior UX & DX:** By generating Blurhash strings and WebP images at upload time, we enable client applications to load faster and show beautiful placeholders instantly. This makes Hermes a highly attractive "Image-Processing-as-a-Service" out of the box.
* **Performance:** Rust is uniquely positioned to handle heavy CPU bounds tasks (like image processing) safely and blazingly fast using crates like `image`.
* **Reduced Infrastructure:** We eliminate the need to maintain, back up, and monitor a separate MinIO cluster. The architecture remains strictly: Rust API <-> Postgres <-> K8s Volumes.

## Rejected Alternatives
* **MinIO / AWS S3:** Rejected because they are "dumb" storage. To get image processing, we would have to build a proxy layer in front of S3 anyway, defeating the purpose of a simple PaaS/BaaS MVP.
* **Direct DB Storage (BLOBs):** Rejected because storing large binaries in PostgreSQL destroys database performance and makes backups unmanageable.

## Consequences
* **Positive:** High value-add for developers. Completely self-contained architecture. Lower bandwidth consumption for hosted apps.
* **Negative:** Image processing is CPU and memory-intensive. We must ensure the Rust backend limits file upload sizes and uses worker threads (e.g., `tokio::spawn` or a queue system) to prevent blocking the main async HTTP loop during heavy concurrent uploads.