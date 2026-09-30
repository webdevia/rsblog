//! Per-user anti-spam: write throttles, duplicate detection, link caps.
//!
//! Complements the per-IP rate limiter (`rate_limiter.rs`): IP buckets stop
//! floods, these checks stop *content* spam account-by-account. State is
//! in-memory sliding windows keyed by user id (no migration needed); a
//! restart resets counters, which only errs toward leniency.
//!
//! Check order inside `check_*_write` is deliberate: content policy (`422`)
//! and duplicates (`409`) are evaluated before the rate quota (`429`), so
//! rejected requests don't burn quota.

use crate::{
    auth::middleware::AuthUser,
    config::Config,
    db::DbPool,
    errors::{AppError, AppResult},
    models::user::Role,
    repositories::user_repo,
};
use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
struct SpamInner {
    /// `"{kind}:{user_id}"` -> write timestamps (sliding window).
    rates: HashMap<String, VecDeque<Instant>>,
    /// `user_id` -> `(content hash, timestamp)` of recent writes.
    dupes: HashMap<String, Vec<(u64, Instant)>>,
}

impl SpamInner {
    /// Sliding-window allowance. Records the attempt on success.
    /// `max == 0` disables the check (always allowed, nothing recorded).
    fn check_rate_at(&mut self, key: &str, max: u32, window: Duration, now: Instant) -> bool {
        if max == 0 {
            return true;
        }
        let q = self.rates.entry(key.to_owned()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= window) {
            q.pop_front();
        }
        if q.len() >= max as usize {
            return false;
        }
        q.push_back(now);
        true
    }

    /// Duplicate detection. Records new content; repeats are not re-recorded.
    /// `window_secs == 0` disables the check.
    fn check_duplicate_at(
        &mut self,
        user_id: &str,
        hash: u64,
        window_secs: i64,
        now: Instant,
    ) -> bool {
        if window_secs <= 0 {
            return false;
        }
        let window = Duration::from_secs(window_secs as u64 * 60);
        let v = self.dupes.entry(user_id.to_owned()).or_default();
        v.retain(|(_, t)| now.duration_since(*t) < window);
        if v.iter().any(|(h, _)| *h == hash) {
            return true;
        }
        v.push((hash, now));
        // Bound memory: keep only the newest entries per user.
        if v.len() > 64 {
            v.drain(..v.len() - 64);
        }
        false
    }
}

#[derive(Debug, Clone, Default)]
pub struct SpamState {
    inner: Arc<Mutex<SpamInner>>,
}

impl SpamState {
    fn lock(&self) -> std::sync::MutexGuard<'_, SpamInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn check_rate(&self, user_id: &str, kind: &str, max: u32, window: Duration) -> bool {
        self.lock()
            .check_rate_at(&format!("{kind}:{user_id}"), max, window, Instant::now())
    }

    pub fn check_duplicate(&self, user_id: &str, hash: u64, window_min: i64) -> bool {
        self.lock()
            .check_duplicate_at(user_id, hash, window_min, Instant::now())
    }
}

/// Hash of the identifying content (title + body) for duplicate detection.
pub fn content_hash(title: &str, content: &str) -> u64 {
    let mut h = DefaultHasher::new();
    title.hash(&mut h);
    content.hash(&mut h);
    h.finish()
}

/// Number of `http://` / `https://` links (case-insensitive, non-overlapping).
pub fn count_links(text: &str) -> usize {
    let lower = text.to_lowercase();
    lower.match_indices("http://").count() + lower.match_indices("https://").count()
}

/// Accounts that skip write throttles: moderators/admins, old accounts, or
/// authors with enough published posts. Thresholds of `0` disable that path.
pub async fn is_trusted(pool: &DbPool, config: &Config, user: &AuthUser) -> AppResult<bool> {
    if user.role.has_permission(&Role::Moderator) {
        return Ok(true);
    }
    let Some(row) = user_repo::find_by_id(pool, &user.id).await? else {
        return Ok(false);
    };
    if config.trusted_account_days > 0
        && account_age_days(&row.created_at) >= config.trusted_account_days
    {
        return Ok(true);
    }
    if config.trusted_published_count > 0
        && user_repo::count_published_by_author(pool, &user.id).await?
            >= config.trusted_published_count
    {
        return Ok(true);
    }
    Ok(false)
}

/// Days since `created_at` (stored TEXT: `"YYYY-MM-DD HH:MM:SS[.frac][tz]"` on
/// both backends — only the leading 19 chars are parsed, as UTC).
/// Unparseable timestamps count as age 0 (fail closed: untrusted).
fn account_age_days(created_at: &str) -> i64 {
    let stamp = created_at.get(..19.min(created_at.len())).unwrap_or("");
    let Ok(naive) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S") else {
        return 0;
    };
    chrono::Utc::now()
        .signed_duration_since(naive.and_utc())
        .num_days()
        .max(0)
}

/// Gate a post write: link cap (`422`) → duplicate (`409`) → rate (`429`).
/// Link/rate checks apply to untrusted users only; duplicates to everyone.
pub async fn check_post_write(
    pool: &DbPool,
    spam: &SpamState,
    config: &Config,
    user: &AuthUser,
    title: &str,
    content: &str,
    excerpt: Option<&str>,
) -> AppResult<()> {
    let trusted = is_trusted(pool, config, user).await?;
    if !trusted && config.max_links_new_user > 0 {
        let links =
            count_links(title) + count_links(content) + excerpt.map(count_links).unwrap_or(0);
        if links > config.max_links_new_user {
            return Err(AppError::Validation(format!(
                "Too many links (max {} for new accounts)",
                config.max_links_new_user
            )));
        }
    }
    if spam.check_duplicate(
        &user.id,
        content_hash(title, content),
        config.duplicate_window_min,
    ) {
        return Err(AppError::Conflict(
            "Duplicate content submitted recently".into(),
        ));
    }
    if !trusted
        && !spam.check_rate(
            &user.id,
            "post",
            config.post_rate_per_hour,
            Duration::from_secs(3600),
        )
    {
        return Err(AppError::TooManyRequests);
    }
    Ok(())
}

/// Gate a comment write: same policy as posts with the comment knobs.
pub async fn check_comment_write(
    pool: &DbPool,
    spam: &SpamState,
    config: &Config,
    user: &AuthUser,
    content: &str,
) -> AppResult<()> {
    let trusted = is_trusted(pool, config, user).await?;
    if !trusted && config.max_links_new_user > 0 && count_links(content) > config.max_links_new_user
    {
        return Err(AppError::Validation(format!(
            "Too many links (max {} for new accounts)",
            config.max_links_new_user
        )));
    }
    if spam.check_duplicate(
        &user.id,
        content_hash("", content),
        config.duplicate_window_min,
    ) {
        return Err(AppError::Conflict(
            "Duplicate content submitted recently".into(),
        ));
    }
    if !trusted
        && !spam.check_rate(
            &user.id,
            "comment",
            config.comment_rate_per_min,
            Duration::from_secs(60),
        )
    {
        return Err(AppError::TooManyRequests);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_counter_cases() {
        assert_eq!(count_links("no links here"), 0);
        assert_eq!(count_links("see https://example.com"), 1);
        assert_eq!(count_links("HTTP://a and http://b and Https://c"), 3);
        // Substring without scheme is not a link.
        assert_eq!(count_links("example.com"), 0);
    }

    #[test]
    fn content_hash_differs_by_input() {
        assert_eq!(content_hash("a", "b"), content_hash("a", "b"));
        assert_ne!(content_hash("a", "b"), content_hash("a", "c"));
        assert_ne!(content_hash("a", "b"), content_hash("b", "b"));
    }

    #[test]
    fn rate_window_allows_then_blocks_then_refills() {
        let mut inner = SpamInner::default();
        let t0 = Instant::now();
        assert!(inner.check_rate_at("k", 2, Duration::from_secs(60), t0));
        assert!(inner.check_rate_at("k", 2, Duration::from_secs(60), t0));
        assert!(!inner.check_rate_at("k", 2, Duration::from_secs(60), t0));
        // After the window passes, quota refills.
        let t1 = t0 + Duration::from_secs(61);
        assert!(inner.check_rate_at("k", 2, Duration::from_secs(60), t1));
        // Other keys are independent.
        assert!(inner.check_rate_at("other", 2, Duration::from_secs(60), t0));
        // Zero disables.
        assert!(inner.check_rate_at("k", 0, Duration::from_secs(60), t0));
    }

    #[test]
    fn duplicate_detection_window() {
        let mut inner = SpamInner::default();
        let t0 = Instant::now();
        assert!(!inner.check_duplicate_at("u", 42, 60, t0));
        assert!(inner.check_duplicate_at("u", 42, 60, t0));
        assert!(!inner.check_duplicate_at("u", 43, 60, t0));
        // Expired entries no longer match.
        let t1 = t0 + Duration::from_secs(3601);
        assert!(!inner.check_duplicate_at("u", 42, 60, t1));
        // Zero window disables.
        assert!(!inner.check_duplicate_at("u", 42, 0, t0));
    }

    #[test]
    fn account_age_parsing() {
        assert!(account_age_days("2020-01-01 00:00:00") > 1000);
        assert_eq!(account_age_days("garbage"), 0);
        assert_eq!(account_age_days(""), 0);
        // Postgres-style fractional + tz suffix: leading 19 chars still parse.
        assert!(account_age_days("2020-01-01 00:00:00.123456+00") > 1000);
    }
}
