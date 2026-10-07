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

/// `TriageVerdict::model` prefix for verdicts from OpenAI's Decisions API.
pub const DECISIONS_MODEL_PREFIX: &str = "decisions:";

/// The Decisions threshold. Its probabilities are a distribution, not a
/// prompted point estimate, so 0.7 means something different there. Measured
/// on 419 real messages against "the recipient replied" (2026-10-06,
/// `examples/triage_compare.rs`): at 0.1 it flags 65 and catches 19 of 26
/// replied-to messages, against prompted Luna's 55 / 17 at 0.7. Retune from the
/// shadow queue, never from a fixture.
pub const DECISIONS_NEEDS_RESPONSE_THRESHOLD: f32 = 0.1;

impl TriageVerdict {
    pub fn is_decisions(&self) -> bool {
        self.model.starts_with(DECISIONS_MODEL_PREFIX)
    }

    /// The threshold belongs to the verdict's origin, so stored Luna verdicts
    /// keep their meaning after the default engine changes.
    pub fn needs_response_threshold(&self) -> f32 {
        if self.is_decisions() {
            DECISIONS_NEEDS_RESPONSE_THRESHOLD
        } else {
            NEEDS_RESPONSE_THRESHOLD
        }
    }

    pub fn says_needs_response(&self) -> bool {
        self.needs_response >= self.needs_response_threshold()
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
