//! The stored result of the AI triage pass, in the shape `needs_you` consumes.
//!
//! Lives in core because both `cxmail-db` (which stores it) and
//! `needs_you::evaluate` (which reads it, and is also here) need the type, and
//! the dependency direction is `core <- db <- email`. The prompt that produces
//! it lives in `cxmail-email::email::triage`; this is only the answer.

use serde::{Deserialize, Serialize};

/// One message's verdict. Every field is the model's raw output, stored
/// unthresholded so a threshold change never needs re-inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriageVerdict {
    /// Probability that a specific person is waiting on a reply from the user.
    pub needs_response: f32,
    /// Probability that a non-reply action (pay, sign, review, attend) is owed.
    pub needs_action: f32,
    /// One of `primary` / `updates` / `social` / `promotions` / `junk` / `unknown`.
    pub category: String,
    /// 0 = never matters … 4 = hours. Integer because the model was asked for
    /// a level, not a number to interpolate between.
    pub urgency: u8,
    /// Which model answered, so a verdict can be trusted or discarded by origin.
    pub model: String,
}

/// `needs_response` at or above this adds the message to Needs You (in `on`
/// mode only). Conservative on purpose: a prompted model returns a point
/// estimate, not a calibrated distribution, so 0.7 here means less than it
/// did from a System One model. Tuned against the shadow-mode queue, never
/// from a fixture.
pub const NEEDS_RESPONSE_THRESHOLD: f32 = 0.7;

impl TriageVerdict {
    pub fn says_needs_response(&self) -> bool {
        self.needs_response >= NEEDS_RESPONSE_THRESHOLD
    }

    pub fn urgency_label(&self) -> &'static str {
        match self.urgency {
            0 => "no rush",
            1 => "whenever",
            2 => "this week",
            3 => "today",
            _ => "now",
        }
    }
}
