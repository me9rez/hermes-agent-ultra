//! Skill write-origin provenance (Python `tools/skill_provenance.py`).

use std::cell::RefCell;

thread_local! {
    static WRITE_ORIGIN: RefCell<String> = RefCell::new("assistant_tool".to_string());
}

pub const BACKGROUND_REVIEW: &str = "background_review";

/// Bind the active write origin for the current thread (foreground turn default).
pub fn set_current_write_origin(origin: &str) {
    let value = if origin.trim().is_empty() {
        "assistant_tool".to_string()
    } else {
        origin.to_string()
    };
    WRITE_ORIGIN.with(|slot| *slot.borrow_mut() = value);
}

/// Read the active write origin.
pub fn get_current_write_origin() -> String {
    WRITE_ORIGIN.with(|slot| slot.borrow().clone())
}

/// True when running inside the background review fork.
pub fn is_background_review_write_origin() -> bool {
    get_current_write_origin() == BACKGROUND_REVIEW
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_origin_roundtrip() {
        set_current_write_origin(BACKGROUND_REVIEW);
        assert_eq!(get_current_write_origin(), BACKGROUND_REVIEW);
        set_current_write_origin("assistant_tool");
        assert!(!is_background_review_write_origin());
    }
}
