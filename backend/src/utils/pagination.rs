//! Shared offset-based pagination for list endpoints.
//!
//! Handlers take `Query(pagination): Query<PaginationParams>` and return
//! `Json(Paginated::new(items, total, page, page_size))`. The frontend reads the
//! `{ items, total, page, pageSize }` envelope.

use serde::{Deserialize, Serialize};

/// Default page size when the client omits `pageSize`.
const DEFAULT_PAGE_SIZE: i64 = 20;
/// Upper bound to protect the server from huge page requests.
const MAX_PAGE_SIZE: i64 = 100;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self { page: None, page_size: None }
    }
}

impl PaginationParams {
    /// Returns the sanitized `(page, page_size, offset)`. `page` is clamped to >= 1
    /// and `page_size` to `[1, MAX_PAGE_SIZE]`.
    pub fn resolve(&self) -> (i64, i64, i64) {
        let page = self.page.unwrap_or(1).max(1);
        let page_size = self.page_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
        let offset = (page - 1) * page_size;
        (page, page_size, offset)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

impl<T> Paginated<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, page_size: i64) -> Self {
        Self { items, total, page, page_size }
    }
}
