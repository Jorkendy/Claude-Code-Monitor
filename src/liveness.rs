// PID liveness via `kill(pid, 0)`: returns 0 if the process exists and we may
// signal it; -1 with errno ESRCH means no such process. We never actually
// signal — sig 0 is a probe.

use crate::model::LiveStatus;

pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}

pub fn classify(pid: Option<i32>, status_field: Option<&str>) -> LiveStatus {
    match pid {
        Some(p) if pid_alive(p) => match status_field {
            Some("idle") => LiveStatus::Idle,
            _ => LiveStatus::Active,
        },
        _ => LiveStatus::Inactive,
    }
}
