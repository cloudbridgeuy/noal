//! What one ask produced, for the page and for the debug panel.
//!
//! The shell fills this in as it goes and hands it to the view once. Nothing
//! here knows how a stage was run; it only records what happened.

use serde::Serialize;

use super::plan::Plan;

/// The steps of the pipeline, in the order they can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The model writes SQL and a shape.
    Plan,
    /// Postgres runs the SQL.
    Query,
    /// The model writes a template.
    Render,
    /// Tera fills the template with the rows.
    Fill,
}

/// A stage that ran and was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StageAttempt {
    /// Which stage.
    pub stage: Stage,
    /// What was tried: SQL, or a template.
    pub artifact: String,
    /// Why it was refused.
    pub error: String,
}

/// How long one stage took, wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Timing {
    /// Which stage.
    pub stage: Stage,
    /// Milliseconds, as measured by the shell.
    pub millis: u64,
}

/// Everything the debug panel shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Debug {
    /// The last plan the model produced, if any.
    pub plan: Option<Plan>,
    /// The last template the model produced, if any.
    pub template: Option<String>,
    /// Every refused attempt, in order.
    pub attempts: Vec<StageAttempt>,
    /// Per-stage durations, in order of completion.
    pub timings: Vec<Timing>,
}

/// How the ask ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The filled template, ready to be placed in the page as-is.
    Answered {
        /// HTML produced by Tera from the model's template and the rows.
        html: String,
    },
    /// Every attempt at some stage was refused.
    Failed {
        /// The stage that gave up.
        stage: Stage,
    },
}

/// Where one ask came from.
///
/// The pipeline reads this to decide its retry policy: an [`Origin::Asked`]
/// ask may send the model back to work, a [`Origin::Reopened`] ask owns its
/// stored plan and template and never calls the model at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The viewer typed a request; the model plans and renders from scratch.
    Asked,
    /// A saved window was reopened; its stored query runs through its stored
    /// template, and a refused stage ends the ask.
    Reopened,
}

/// One ask, start to finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// What the user typed.
    pub request: String,
    /// How it ended.
    pub verdict: Verdict,
    /// Whether the model was ever in the loop.
    pub origin: Origin,
    /// What the debug panel shows.
    pub debug: Debug,
}

impl Outcome {
    /// The debug payload as JSON, safe to place inside a `<script>` element.
    ///
    /// A `</script>` inside a string value would end the element early, so
    /// `</` is written as `<\/`, which JSON reads back unchanged.
    #[must_use]
    pub fn debug_json(&self) -> String {
        let payload = DebugPayload {
            request: &self.request,
            failed_stage: match &self.verdict {
                Verdict::Answered { .. } => None,
                Verdict::Failed { stage } => Some(*stage),
            },
            origin: self.origin,
            debug: &self.debug,
        };
        serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{}".to_owned())
            .replace("</", "<\\/")
    }
}

/// The wire shape of the debug panel data.
#[derive(Serialize)]
struct DebugPayload<'a> {
    /// What the user typed.
    request: &'a str,
    /// The stage that gave up, if any.
    failed_stage: Option<Stage>,
    /// Whether the model was ever in the loop.
    origin: Origin,
    /// The plan, template, attempts, and timings.
    #[serde(flatten)]
    debug: &'a Debug,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{Debug, Origin, Outcome, Stage, StageAttempt, Timing, Verdict};

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            origin: Origin::Asked,
            debug: Debug {
                plan: None,
                template: Some("<p>{{ rows | length }}</script>".into()),
                attempts: vec![StageAttempt {
                    stage: Stage::Query,
                    artifact: "select nope".into(),
                    error: "boom".into(),
                }],
                timings: vec![Timing {
                    stage: Stage::Plan,
                    millis: 1200,
                }],
            },
        }
    }

    #[test]
    fn debug_json_carries_request_attempts_and_timings() {
        let json = outcome(Verdict::Answered {
            html: String::new(),
        })
        .debug_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["request"], "open tasks");
        assert_eq!(value["attempts"][0]["stage"], "query");
        assert_eq!(value["timings"][0]["millis"], 1200);
        assert!(value["failed_stage"].is_null());
    }

    #[test]
    fn debug_json_names_the_failed_stage() {
        let json = outcome(Verdict::Failed { stage: Stage::Fill }).debug_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["failed_stage"], "fill");
    }

    #[test]
    fn debug_json_names_where_the_ask_came_from() {
        let asked = outcome(Verdict::Answered {
            html: String::new(),
        })
        .debug_json();
        let value: serde_json::Value = serde_json::from_str(&asked).unwrap();
        assert_eq!(value["origin"], "asked");

        let reopened = Outcome {
            origin: Origin::Reopened,
            ..outcome(Verdict::Failed {
                stage: Stage::Query,
            })
        };
        let value: serde_json::Value = serde_json::from_str(&reopened.debug_json()).unwrap();
        assert_eq!(value["origin"], "reopened");
    }

    #[test]
    fn debug_json_cannot_close_a_script_element() {
        let json = outcome(Verdict::Answered {
            html: String::new(),
        })
        .debug_json();
        assert!(!json.contains("</script>"));
        assert!(json.contains("<\\/script>"));
        // And it is still valid JSON with the original text inside.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["template"].as_str().unwrap().contains("</script>"));
    }
}
