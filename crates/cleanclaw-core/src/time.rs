use chrono::{DateTime, Utc};

#[inline]
pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
