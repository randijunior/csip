use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn new_session_id() -> u64 {
    todo!()
}

pub(crate) fn new_session_version() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .subsec_nanos() as u64
}
