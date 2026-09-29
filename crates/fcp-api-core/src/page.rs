//! One envelope for every list (docs/DEVELOPER_SURFACE.md §3, phase 2c).
//!
//! A collection answers `{"data": [...], "next_cursor": "…" | null,
//! "total": n}` and takes `?limit=` (default [`DEFAULT_LIMIT`], at most
//! [`MAX_LIMIT`]) and `?cursor=` (the `next_cursor` of the page before).
//! `total` is present where it is known.
//!
//! Pages are keyset pages: each list is ordered by a stable key — a name,
//! a number, an id, or a time with the id to break ties — and a cursor
//! names the last key returned, so a page never repeats or skips a row
//! because another was added or removed before it. A cursor is opaque to
//! the caller.

use crate::error::ApiError;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Items a page holds when the caller does not say.
pub const DEFAULT_LIMIT: usize = 100;
/// The most items one page may hold.
pub const MAX_LIMIT: usize = 1000;

/// `?limit=&cursor=`.
#[derive(Debug, Default, Clone, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl PageQuery {
    /// The page size asked for, checked.
    pub fn limit(&self) -> Result<usize, ApiError> {
        match self.limit {
            None => Ok(DEFAULT_LIMIT),
            Some(0) => Err(ApiError::BadRequest("limit must be at least 1".into())),
            Some(n) if n > MAX_LIMIT => Err(ApiError::BadRequest(format!(
                "limit may be at most {MAX_LIMIT}"
            ))),
            Some(n) => Ok(n),
        }
    }

    /// The key the cursor names, if one was given.
    pub fn after(&self) -> Result<Option<String>, ApiError> {
        match self.cursor.as_deref().filter(|c| !c.is_empty()) {
            None => Ok(None),
            Some(c) => decode(c).map(Some),
        }
    }
}

impl PageQuery {
    /// Where an offset page starts: 0, or the position its cursor names.
    /// For the few lists a store pages by offset — flows, recordings, flow
    /// traces, billing transactions — whose growth is bounded or slow.
    pub fn offset(&self) -> Result<usize, ApiError> {
        match self.after()? {
            None => Ok(0),
            Some(key) => key
                .strip_prefix("o:")
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| ApiError::BadRequest("Invalid cursor".into())),
        }
    }
}

/// An offset page: `data` from `offset`, and a cursor to the next when
/// there is `more`.
pub fn offset_page<T>(data: Vec<T>, offset: usize, more: bool, total: Option<u64>) -> Page<T> {
    let next = offset + data.len();
    Page::new(data, more.then(|| format!("o:{next}")), total)
}

/// A page of a list.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Page<T> {
    pub data: Vec<T>,
    /// The cursor for the next page; null on the last.
    pub next_cursor: Option<String>,
    /// How many items the whole list holds, where that is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> Page<T> {
    /// A page built by the caller (a store that pages itself).
    pub fn new(data: Vec<T>, next_key: Option<String>, total: Option<u64>) -> Self {
        Self {
            data,
            next_cursor: next_key.map(|k| encode(&k)),
            total,
        }
    }
}

/// Which way a list's key runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Smallest key first: names, numbers, ids.
    Ascending,
    /// Largest key first: newest first, for anything keyed by time.
    Descending,
}

/// Page a whole list held in memory: order it by `key`, skip up to the
/// cursor's key, take `limit`.
pub fn paginate<T>(
    mut items: Vec<T>,
    query: &PageQuery,
    order: Order,
    key: impl Fn(&T) -> String,
) -> Result<Page<T>, ApiError> {
    let limit = query.limit()?;
    let after = query.after()?;
    let total = items.len() as u64;
    let mut keyed: Vec<(String, T)> = items.drain(..).map(|t| (key(&t), t)).collect();
    keyed.sort_by(|a, b| match order {
        Order::Ascending => a.0.cmp(&b.0),
        Order::Descending => b.0.cmp(&a.0),
    });
    let start = match &after {
        None => 0,
        Some(after) => keyed.partition_point(|(k, _)| match order {
            Order::Ascending => k <= after,
            Order::Descending => k >= after,
        }),
    };
    let mut rest = keyed.into_iter().skip(start);
    let data: Vec<(String, T)> = rest.by_ref().take(limit).collect();
    let more = rest.next().is_some();
    let next_key = if more {
        data.last().map(|(k, _)| k.clone())
    } else {
        None
    };
    Ok(Page::new(
        data.into_iter().map(|(_, t)| t).collect(),
        next_key,
        Some(total),
    ))
}

/// A key for a row ordered by a `SystemTime`, to the nanosecond.
pub fn system_time_key(at: std::time::SystemTime, tie: &str) -> String {
    let nanos = at
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:032}/{tie}")
}

/// A key for a row ordered by an RFC 3339 time already formatted.
pub fn rfc3339_key(at: &str, tie: &str) -> String {
    let nanos = chrono::DateTime::parse_from_rfc3339(at)
        .map(|dt| dt.timestamp_nanos_opt().unwrap_or(0).max(0) as u128)
        .unwrap_or(0);
    format!("{nanos:032}/{tie}")
}

/// A key for a time-ordered row: the time, zero-padded so it sorts as a
/// number, then something unique to break ties.
pub fn time_key(epoch: u64, tie: &str) -> String {
    format!("{epoch:020}/{tie}")
}

fn encode(key: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_bytes())
}

fn decode(cursor: &str) -> Result<String, ApiError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(|| ApiError::BadRequest("Invalid cursor".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(limit: usize, cursor: Option<String>) -> PageQuery {
        PageQuery {
            limit: Some(limit),
            cursor,
        }
    }

    #[test]
    fn pages_walk_the_whole_list_once_in_order() {
        let items: Vec<u32> = vec![5, 3, 9, 1, 7];
        let key = |n: &u32| format!("{n:03}");
        let first = paginate(items.clone(), &q(2, None), Order::Ascending, key).unwrap();
        assert_eq!(first.data, vec![1, 3]);
        assert_eq!(first.total, Some(5));
        let second = paginate(
            items.clone(),
            &q(2, first.next_cursor),
            Order::Ascending,
            key,
        )
        .unwrap();
        assert_eq!(second.data, vec![5, 7]);
        let third = paginate(
            items.clone(),
            &q(2, second.next_cursor),
            Order::Ascending,
            key,
        )
        .unwrap();
        assert_eq!(third.data, vec![9]);
        assert!(third.next_cursor.is_none());

        let newest = paginate(items, &q(3, None), Order::Descending, key).unwrap();
        assert_eq!(newest.data, vec![9, 7, 5]);
    }

    #[test]
    fn a_row_added_before_the_cursor_neither_repeats_nor_skips() {
        let key = |n: &u32| format!("{n:03}");
        let first = paginate(vec![1, 3, 5, 7], &q(2, None), Order::Ascending, key).unwrap();
        assert_eq!(first.data, vec![1, 3]);
        // 2 arrives, 1 leaves, between the pages.
        let second = paginate(
            vec![2, 3, 5, 7],
            &q(2, first.next_cursor),
            Order::Ascending,
            key,
        )
        .unwrap();
        assert_eq!(second.data, vec![5, 7]);
    }

    #[test]
    fn offset_cursors_carry_the_position() {
        let first = offset_page(vec![1, 2], 0, true, Some(5));
        let q = q(2, first.next_cursor);
        assert_eq!(q.offset().unwrap(), 2);
        let last = offset_page(vec![5], 4, false, Some(5));
        assert!(last.next_cursor.is_none());
    }

    #[test]
    fn limits_and_cursors_are_checked() {
        assert!(q(0, None).limit().is_err());
        assert!(q(MAX_LIMIT + 1, None).limit().is_err());
        assert_eq!(PageQuery::default().limit().unwrap(), DEFAULT_LIMIT);
        assert!(q(1, Some("%%%".into())).after().is_err());
    }
}
