use serde::Deserialize;
use utoipa::IntoParams;

/// Shared `?limit=&offset=` pagination for list endpoints.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Pagination {
    /// Page size, 1-100 (default 20).
    pub limit: Option<i64>,
    /// Number of rows to skip (default 0).
    pub offset: Option<i64>,
}

impl Pagination {
    /// Clamps user input so a list endpoint can never scan an unbounded table.
    pub fn normalize(&self) -> (i64, i64) {
        let limit = self.limit.unwrap_or(20).clamp(1, 100);
        let offset = self.offset.unwrap_or(0).max(0);
        (limit, offset)
    }
}
