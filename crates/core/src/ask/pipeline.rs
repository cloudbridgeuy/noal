//! The ask pipeline as a pure state machine.
//!
//! [`Pipeline`] decides what runs next; it never runs anything itself. The
//! shell drives it: it starts a [`Pipeline`], runs whatever [`Step`]s come
//! back, turns each result into an [`Event`], and feeds the event back in.
//! That split means the policy — what follows a plan, what a refused query
//! means, when to give up — lives here as ordinary code with ordinary tests,
//! and the shell's loop never has to know why it is doing what it does.

use super::outcome::{Debug, Origin, Outcome, Stage, StageAttempt, Timing, Verdict};
use super::plan::{wrap_sql, Parent, Plan};
use super::prompt::{plan_prompt, render_prompt, Attempt, MAX_ATTEMPTS};
use super::validator::forbidden_token;

/// One thing the shell must run before the pipeline can continue.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Ask the model to plan: write SQL and describe its shape.
    Plan {
        /// The user message for the planning call.
        prompt: String,
    },
    /// Run this SQL, already wrapped for a single JSON row.
    Query {
        /// The wrapped `SELECT`, ready for `simple_query`.
        sql: String,
    },
    /// Ask the model to write a template for this shape.
    Render {
        /// The user message for the rendering call.
        prompt: String,
    },
    /// Fill this template with these rows.
    Fill {
        /// The template text, fences already stripped.
        template: String,
        /// The rows, as the query returned them.
        rows: serde_json::Value,
    },
    /// The ask is over. Hand this to the view.
    Done(Outcome),
}

/// What the shell learned after running a [`Step`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The model's plan.
    Planned(Plan),
    /// The query's rows, or the message Postgres refused it with.
    Queried(Result<serde_json::Value, String>),
    /// The model's template, with any surrounding code fence already
    /// stripped.
    Rendered(String),
    /// The filled page, or the report Tera refused it with.
    Filled(Result<String, String>),
}

/// The ask pipeline's state between steps.
///
/// A query and a render are asked for together, and each can finish before
/// the other. [`Pipeline`] holds whichever result has arrived and issues the
/// fill step exactly once, the moment both are in hand.
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// What the user typed.
    request: String,
    /// The plan the model produced, once it has.
    plan: Option<Plan>,
    /// The rows the query returned, once it has.
    rows: Option<serde_json::Value>,
    /// The template the model produced, once it has.
    template: Option<String>,
    /// Set once a fill step has been issued, so a second arrival of the other
    /// half of the pair does not issue a second one.
    filled: bool,
    /// Refused plans, oldest first. Read by the retry policy to decide
    /// whether another `Plan` step is owed, and passed to `plan_prompt` so
    /// the model sees what it tried and why it was refused.
    plan_attempts: Vec<Attempt>,
    /// Refused templates, oldest first. Read by the retry policy to decide
    /// whether another `Render` step is owed, and passed to `render_prompt`
    /// the same way.
    render_attempts: Vec<Attempt>,
    /// What the debug panel will show.
    debug: Debug,
    /// Whether the model was ever in the loop. A reopened pipeline owns its
    /// plan and template, so its retry hooks refuse to call the model again.
    origin: Origin,
    /// The window this ask refines, when the address bar named one. Held for
    /// the whole ask so both prompts draw on it and a retry re-uses it.
    parent: Option<Parent>,
}

impl Pipeline {
    /// Start an ask: the first step is always to plan.
    ///
    /// `parent` is the window the address bar names, or `None` at the root:
    /// a follow-up is not a different machine from an ask, only an argument
    /// richer one.
    #[must_use]
    pub fn start(request: String, parent: Option<Parent>) -> (Self, Vec<Step>) {
        let prompt = plan_prompt(&request, parent.as_ref(), &[]);
        let pipeline = Self {
            request,
            plan: None,
            rows: None,
            template: None,
            filled: false,
            plan_attempts: Vec::new(),
            render_attempts: Vec::new(),
            debug: Debug::default(),
            origin: Origin::Asked,
            parent,
        };
        (pipeline, vec![Step::Plan { prompt }])
    }

    /// Reopen a saved window: run its stored plan's query through its stored
    /// template.
    ///
    /// The plan and template are set up front, so the first step is the
    /// query — no planning call, no rendering call. Should the query or the
    /// fill be refused there is nothing to retry with: the ask ends rather
    /// than reaching for the model, which is what [`Origin::Reopened`] means.
    #[must_use]
    pub fn reopen(request: String, plan: Plan, template: String) -> (Self, Vec<Step>) {
        let sql = wrap_sql(&plan.sql);
        let debug = Debug {
            plan: Some(plan.clone()),
            template: Some(template.clone()),
            ..Debug::default()
        };
        let pipeline = Self {
            request,
            plan: Some(plan),
            rows: None,
            template: Some(template),
            filled: false,
            plan_attempts: Vec::new(),
            render_attempts: Vec::new(),
            debug,
            origin: Origin::Reopened,
            parent: None,
        };
        (pipeline, vec![Step::Query { sql }])
    }

    /// Record how long a stage took, for the debug panel.
    ///
    /// The shell measures every stage it runs, including a query and a
    /// render that ran concurrently, and calls this once per stage. It is
    /// pure bookkeeping: nothing here reads a clock, so the timing itself is
    /// always a value the shell already computed.
    pub fn record(&mut self, timing: Timing) {
        self.debug.timings.push(timing);
    }

    /// Advance the pipeline with one result from the shell.
    ///
    /// Returns the steps that follow, which may be empty (still waiting on
    /// the other half of a query/render pair) or end in [`Step::Done`].
    #[must_use]
    pub fn apply(&mut self, event: Event) -> Vec<Step> {
        match event {
            Event::Planned(plan) => self.on_planned(plan),
            Event::Queried(Ok(rows)) => self.on_queried_ok(rows),
            Event::Queried(Err(message)) => self.on_queried_err(message),
            Event::Rendered(text) => self.on_rendered(text),
            Event::Filled(Ok(html)) => self.on_filled_ok(html),
            Event::Filled(Err(error)) => self.on_filled_err(error),
        }
    }

    /// A plan arrived: query it and, unless a template already fits its
    /// shape, ask for one.
    ///
    /// A second `Plan` event, arriving after the first query was refused,
    /// finds stale `rows` and a `filled` latch from the attempt that never
    /// completed. Both are reset here so the query this plan starts can
    /// still reach a fill, whether or not the held template survives.
    fn on_planned(&mut self, plan: Plan) -> Vec<Step> {
        let sql = wrap_sql(&plan.sql);
        let reuses_template = self.template.is_some()
            && self
                .plan
                .as_ref()
                .is_some_and(|previous| previous.same_shape(&plan));

        self.debug.plan = Some(plan.clone());
        self.rows = None;
        self.filled = false;

        if reuses_template {
            self.plan = Some(plan);
            vec![Step::Query { sql }]
        } else {
            // The held template, if any, was written for a shape this plan
            // does not share; it would bind to the wrong fields.
            self.template = None;
            let prompt = render_prompt(
                &self.request,
                &plan.shape,
                self.parent.as_ref(),
                &self.render_attempts,
            );
            self.plan = Some(plan);
            vec![Step::Query { sql }, Step::Render { prompt }]
        }
    }

    /// The query answered: keep the rows, and fill if a template is waiting.
    ///
    /// A reopened ask gates the fill on the shape check first. Its query ran
    /// and did its job; a mismatch is between the stored template's
    /// expectations and the returned columns, so it fails at [`Stage::Fill`],
    /// loudly and with the diff recorded — never as a page of blank cells
    /// that looks like an empty result. A first ask is not gated: the model
    /// still holds the plan it wrote seconds ago.
    fn on_queried_ok(&mut self, rows: serde_json::Value) -> Vec<Step> {
        if self.origin == Origin::Reopened {
            let shape = self
                .plan
                .as_ref()
                .map_or(&[][..], |plan| plan.shape.as_slice());
            if !crate::window::rows_match_shape(&rows, shape) {
                let sql = self
                    .plan
                    .as_ref()
                    .map_or_else(String::new, |plan| plan.sql.clone());
                let error = format!(
                    "the returned columns {returned:?} do not fit the stored shape {stored:?}",
                    returned = returned_column_names(&rows),
                    stored = stored_column_names(shape),
                );
                self.debug.attempts.push(StageAttempt {
                    stage: Stage::Fill,
                    artifact: sql,
                    error,
                });
                return vec![Step::Done(
                    self.finish(Verdict::Failed { stage: Stage::Fill }),
                )];
            }
        }
        self.rows = Some(rows);
        self.try_fill()
    }

    /// The query was refused. Ask the model to plan again, with this attempt
    /// added to the prompt, until `MAX_ATTEMPTS` is spent; then give up.
    fn on_queried_err(&mut self, message: String) -> Vec<Step> {
        let sql = self
            .plan
            .as_ref()
            .map_or_else(String::new, |plan| plan.sql.clone());
        self.debug.attempts.push(StageAttempt {
            stage: Stage::Query,
            artifact: sql.clone(),
            error: message.clone(),
        });
        self.plan_attempts.push(Attempt {
            artifact: sql,
            error: message,
        });

        if self.origin == Origin::Reopened {
            // The SQL is the window's own, stored when it was saved. There
            // is no planner to fix it, so a refusal ends the ask instead of
            // becoming another model call.
            return vec![Step::Done(self.finish(Verdict::Failed {
                stage: Stage::Query,
            }))];
        }
        if self.plan_attempts.len() < MAX_ATTEMPTS {
            let prompt = plan_prompt(&self.request, self.parent.as_ref(), &self.plan_attempts);
            vec![Step::Plan { prompt }]
        } else {
            vec![Step::Done(self.finish(Verdict::Failed {
                stage: Stage::Query,
            }))]
        }
    }

    /// The template arrived: keep it, and fill if the rows are waiting.
    fn on_rendered(&mut self, text: String) -> Vec<Step> {
        self.debug.template = Some(text.clone());
        self.template = Some(text);
        self.try_fill()
    }

    /// Tera refused the template. Ask the model to render again, with this
    /// attempt added to the prompt, until `MAX_ATTEMPTS` is spent; then give
    /// up. The rows are already in hand, so the retry only re-renders; it
    /// clears the `filled` latch so the fill this render leads to is not
    /// mistaken for a repeat of the one that just failed.
    fn on_filled_err(&mut self, error: String) -> Vec<Step> {
        let template = self.template.clone().unwrap_or_default();
        self.debug.attempts.push(StageAttempt {
            stage: Stage::Fill,
            artifact: template.clone(),
            error: error.clone(),
        });
        self.render_attempts.push(Attempt {
            artifact: template,
            error,
        });

        if self.origin == Origin::Reopened {
            // The template is the window's own, stored when it was saved.
            // There is no renderer to rewrite it, so a refusal ends the ask
            // instead of becoming another model call.
            return vec![Step::Done(
                self.finish(Verdict::Failed { stage: Stage::Fill }),
            )];
        }
        if self.render_attempts.len() < MAX_ATTEMPTS {
            self.filled = false;
            let shape = self
                .plan
                .as_ref()
                .map_or(&[][..], |plan| plan.shape.as_slice());
            let prompt = render_prompt(
                &self.request,
                shape,
                self.parent.as_ref(),
                &self.render_attempts,
            );
            vec![Step::Render { prompt }]
        } else {
            vec![Step::Done(
                self.finish(Verdict::Failed { stage: Stage::Fill }),
            )]
        }
    }

    /// The fill succeeded: keep the HTML only if it carries no navigation,
    /// no script, no fetch, and no handler token — links, forms/iframes,
    /// CSS `url()`/`@import`, and `hx-*`/`on*` attributes are all covered
    /// by the scan.
    ///
    /// The scan runs on the final HTML because `{{ row.body | safe }}` can
    /// carry a link out of the database past any scan of the template
    /// source. A trip is refused exactly as a Tera refusal is: the attempt
    /// is recorded against [`Stage::Render`] — the fix is a re-render, so
    /// there is no new stage — the retry prompt carries the TEMPLATE (the
    /// only thing the model can change) and names the token found in the
    /// output, and under [`MAX_ATTEMPTS`] the `filled` latch is cleared so
    /// the retry's fill can still be issued.
    fn on_filled_ok(&mut self, html: String) -> Vec<Step> {
        let Some(token) = forbidden_token(&html) else {
            return vec![Step::Done(self.finish(Verdict::Answered { html }))];
        };
        let template = self.template.clone().unwrap_or_default();
        let error = format!("rendered output carries a forbidden token: {token}");
        self.debug.attempts.push(StageAttempt {
            stage: Stage::Render,
            artifact: template.clone(),
            error: error.clone(),
        });
        self.render_attempts.push(Attempt {
            artifact: template,
            error,
        });

        if self.render_attempts.len() < MAX_ATTEMPTS {
            self.filled = false;
            let shape = self
                .plan
                .as_ref()
                .map_or(&[][..], |plan| plan.shape.as_slice());
            let prompt = render_prompt(
                &self.request,
                shape,
                self.parent.as_ref(),
                &self.render_attempts,
            );
            vec![Step::Render { prompt }]
        } else {
            vec![Step::Done(self.finish(Verdict::Failed {
                stage: Stage::Render,
            }))]
        }
    }

    /// Issue the fill step the moment both halves of a query/render pair are
    /// in hand, and never more than once for the same pair.
    fn try_fill(&mut self) -> Vec<Step> {
        if self.filled {
            return Vec::new();
        }
        match (&self.rows, &self.template) {
            (Some(rows), Some(template)) => {
                self.filled = true;
                vec![Step::Fill {
                    template: template.clone(),
                    rows: rows.clone(),
                }]
            }
            _ => Vec::new(),
        }
    }

    /// Close out the ask with the debug record gathered so far.
    fn finish(&self, verdict: Verdict) -> Outcome {
        Outcome {
            request: self.request.clone(),
            verdict,
            origin: self.origin,
            debug: self.debug.clone(),
        }
    }
}

/// The keys of the query's first row, as the diff text names them.
fn returned_column_names(rows: &serde_json::Value) -> Vec<String> {
    rows.as_array()
        .and_then(|all| all.first())
        .and_then(|row| row.as_object())
        .map_or_else(Vec::new, |row| row.keys().cloned().collect())
}

/// The column names of the stored shape, as the diff text names them.
fn stored_column_names(shape: &[super::plan::Column]) -> Vec<String> {
    shape.iter().map(|column| column.name.clone()).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "pipeline/tests.rs"]
mod tests;
