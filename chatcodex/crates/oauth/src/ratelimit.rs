//! Simple in-memory sliding-window rate limiter for the token endpoint.
//!
//! This prevents brute-force attacks on the authorization-code and
//! refresh-token grants. Each client_id is limited to a configurable
//! number of requests per window.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// Rate limiter state shared across requests.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

struct RateLimiterInner {
    /// Map from client_id → (window_start, count)
    windows: HashMap<String, (Instant, u64)>,
    /// Max requests per window per client_id.
    max_per_window: u64,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// `max_per_window` is the maximum number of requests allowed from a
    /// single client_id within `window_secs` seconds.
    pub fn new(max_per_window: u64, window_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                windows: HashMap::new(),
                max_per_window,
                window_secs,
            })),
        }
    }

    /// Check whether a request from `client_id` should be allowed.
    /// Returns `true` if the request is within the rate limit.
    pub fn check(&self, client_id: &str) -> bool {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return true, // allow on lock contention
        };
        let now = Instant::now();
        let window_dur = std::time::Duration::from_secs(guard.window_secs);

        let max = guard.max_per_window;
        let entry = guard
            .windows
            .entry(client_id.to_string())
            .or_insert_with(|| (now, 0));
        if now.duration_since(entry.0) > window_dur {
            // Window expired, reset.
            *entry = (now, 1);
            true
        } else if entry.1 < max {
            entry.1 += 1;
            true
        } else {
            false
        }
    }

    /// Periodic cleanup of expired windows to prevent memory leaks.
    pub fn cleanup(&self) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let now = Instant::now();
        let window_dur = std::time::Duration::from_secs(guard.window_secs);
        guard
            .windows
            .retain(|_, (window_start, _)| now.duration_since(*window_start) <= window_dur);
    }
}

/// Axum middleware that rate-limits requests based on the `client_id` form
/// field. Returns 429 Too Many Requests when the limit is exceeded.
pub async fn rate_limit_token(
    State(limiter): State<RateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Extract client_id from the form body. We need to read the body,
    // parse it, then reconstruct it for downstream handlers.
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 16)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Parse the form data to extract client_id.
    let form_str = String::from_utf8_lossy(&bytes);
    let client_id = form_str.split('&').find_map(|pair| {
        let mut iter = pair.splitn(2, '=');
        match (iter.next(), iter.next()) {
            (Some("client_id"), Some(value)) => Some(urlencoding::decode(value).ok()?.into_owned()),
            _ => None,
        }
    });

    if let Some(client_id) = client_id
        && !limiter.check(&client_id)
    {
        tracing::warn!(client_id = %client_id, "rate limit exceeded on token endpoint");
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    // Reconstruct the request with the original body.
    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_rate_limiting() {
        let limiter = RateLimiter::new(3, 60);
        assert!(limiter.check("client-1"));
        assert!(limiter.check("client-1"));
        assert!(limiter.check("client-1"));
        assert!(!limiter.check("client-1")); // 4th request in window
        assert!(limiter.check("client-2")); // different client, fresh window
    }

    #[test]
    fn window_expires() {
        let limiter = RateLimiter::new(1, 1); // 1 request per 1-second window
        assert!(limiter.check("client-1"));
        assert!(!limiter.check("client-1"));
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(limiter.check("client-1")); // window expired
    }

    #[test]
    fn cleanup_removes_expired_entries() {
        let limiter = RateLimiter::new(1, 0); // 0-second window = always expired
        let _ = limiter.check("client-1");
        limiter.cleanup();
        let guard = limiter.inner.lock().unwrap();
        assert!(guard.windows.is_empty());
    }
}
