//! The ask pipeline as a pure state machine.
//!
//! [`Pipeline`] decides what runs next; it never runs anything itself. The
//! shell drives it: it starts a [`Pipeline`], runs whatever [`Step`]s come
//! back, turns each result into an [`Event`], and feeds the event back in.
//! That split means the policy — what follows a plan, what a refused query
//! means, when to give up — lives here as ordinary code with ordinary tests,
//! and the shell's loop never has to know why it is doing what it does.

use super::outcome::{Debug, Outcome, Stage, StageAttempt, Verdict};
use super::plan::{wrap_sql, Plan};
use super::prompt::{plan_prompt, render_prompt, Attempt};

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
    /// Refused plans, for a future retry policy to read.
    plan_attempts: Vec<Attempt>,
    /// Refused templates, for a future retry policy to read.
    render_attempts: Vec<Attempt>,
    /// What the debug panel will show.
    debug: Debug,
}

impl Pipeline {
    /// Start an ask: the first step is always to plan.
    #[must_use]
    pub fn start(request: String) -> (Self, Vec<Step>) {
        let prompt = plan_prompt(&request, &[]);
        let pipeline = Self {
            request,
            plan: None,
            rows: None,
            template: None,
            filled: false,
            plan_attempts: Vec::new(),
            render_attempts: Vec::new(),
            debug: Debug::default(),
        };
        (pipeline, vec![Step::Plan { prompt }])
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
            Event::Filled(Ok(html)) => vec![Step::Done(self.finish(Verdict::Answered { html }))],
            Event::Filled(Err(error)) => self.on_filled_err(error),
        }
    }

    /// A plan arrived: query it and, unless a template already fits its
    /// shape, ask for one.
    fn on_planned(&mut self, plan: Plan) -> Vec<Step> {
        let sql = wrap_sql(&plan.sql);
        let prompt = render_prompt(&self.request, &plan.shape, &self.render_attempts);
        self.debug.plan = Some(plan.clone());
        self.plan = Some(plan);
        vec![Step::Query { sql }, Step::Render { prompt }]
    }

    /// The query answered: keep the rows, and fill if a template is waiting.
    fn on_queried_ok(&mut self, rows: serde_json::Value) -> Vec<Step> {
        self.rows = Some(rows);
        self.try_fill()
    }

    /// The query was refused. One attempt: the ask ends here.
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
        vec![Step::Done(self.finish(Verdict::Failed {
            stage: Stage::Query,
        }))]
    }

    /// The template arrived: keep it, and fill if the rows are waiting.
    fn on_rendered(&mut self, text: String) -> Vec<Step> {
        self.debug.template = Some(text.clone());
        self.template = Some(text);
        self.try_fill()
    }

    /// Tera refused the template. One attempt: the ask ends here.
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
        vec![Step::Done(
            self.finish(Verdict::Failed { stage: Stage::Fill }),
        )]
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
            debug: self.debug.clone(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{Event, Pipeline, Step};
    use crate::ask::outcome::{Stage, Verdict};
    use crate::ask::plan::{Column, ColumnKind, Plan};
    use serde_json::json;

    fn plan() -> Plan {
        Plan {
            sql: "select id from ticket".into(),
            shape: vec![Column {
                name: "id".into(),
                kind: ColumnKind::Integer,
                description: String::new(),
                fields: Vec::new(),
            }],
        }
    }

    fn done_outcome(steps: &[Step]) -> &super::super::outcome::Outcome {
        match steps {
            [Step::Done(outcome)] => outcome,
            other => panic!("expected exactly one Done step, got {other:?}"),
        }
    }

    #[test]
    fn starting_asks_the_model_to_plan() {
        let (_, steps) = Pipeline::start("open tasks".into());
        assert!(matches!(steps.as_slice(), [Step::Plan { .. }]));
    }

    #[test]
    fn a_plan_issues_a_query_and_a_render_together() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let steps = pipeline.apply(Event::Planned(plan()));
        assert!(matches!(
            steps.as_slice(),
            [Step::Query { .. }, Step::Render { .. }]
        ));
    }

    #[test]
    fn happy_path_fills_once_the_query_answers_first() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let _ = pipeline.apply(Event::Planned(plan()));

        let rows = json!([{ "id": 1 }]);
        let after_query = pipeline.apply(Event::Queried(Ok(rows.clone())));
        assert!(after_query.is_empty(), "still waiting on the render");

        let after_render = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
        match after_render.as_slice() {
            [Step::Fill {
                template,
                rows: filled_rows,
            }] => {
                assert_eq!(template, "<p>{{ rows | length }}</p>");
                assert_eq!(filled_rows, &rows);
            }
            other => panic!("expected exactly one Fill step, got {other:?}"),
        }

        let done = pipeline.apply(Event::Filled(Ok("<p>1</p>".into())));
        let outcome = done_outcome(&done);
        assert_eq!(
            outcome.verdict,
            Verdict::Answered {
                html: "<p>1</p>".into()
            }
        );
    }

    #[test]
    fn happy_path_fills_once_the_render_answers_first() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let _ = pipeline.apply(Event::Planned(plan()));

        let after_render = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
        assert!(after_render.is_empty(), "still waiting on the query");

        let rows = json!([{ "id": 1 }]);
        let after_query = pipeline.apply(Event::Queried(Ok(rows.clone())));
        match after_query.as_slice() {
            [Step::Fill {
                template,
                rows: filled_rows,
            }] => {
                assert_eq!(template, "<p>{{ rows | length }}</p>");
                assert_eq!(filled_rows, &rows);
            }
            other => panic!("expected exactly one Fill step, got {other:?}"),
        }

        let done = pipeline.apply(Event::Filled(Ok("<p>1</p>".into())));
        let outcome = done_outcome(&done);
        assert_eq!(
            outcome.verdict,
            Verdict::Answered {
                html: "<p>1</p>".into()
            }
        );
    }

    #[test]
    fn a_refused_query_ends_the_ask_without_waiting_for_the_render() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let _ = pipeline.apply(Event::Planned(plan()));

        let done = pipeline.apply(Event::Queried(Err("column \"nope\" does not exist".into())));
        let outcome = done_outcome(&done);
        assert_eq!(
            outcome.verdict,
            Verdict::Failed {
                stage: Stage::Query
            }
        );
        assert_eq!(outcome.debug.attempts.len(), 1);
        assert_eq!(outcome.debug.attempts[0].stage, Stage::Query);

        // A late-arriving render must not turn into a second Fill or Done.
        let after_render = pipeline.apply(Event::Rendered("<p></p>".into()));
        assert!(after_render.is_empty());
    }

    #[test]
    fn a_refused_fill_ends_the_ask() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let _ = pipeline.apply(Event::Planned(plan()));
        let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
        let _ = pipeline.apply(Event::Rendered("{{ rows.0.missing }}".into()));

        let done = pipeline.apply(Event::Filled(Err("variable `missing` not found".into())));
        let outcome = done_outcome(&done);
        assert_eq!(outcome.verdict, Verdict::Failed { stage: Stage::Fill });
        assert_eq!(outcome.debug.attempts.len(), 1);
        assert_eq!(outcome.debug.attempts[0].stage, Stage::Fill);
    }

    #[test]
    fn the_fill_step_is_never_issued_twice_for_one_pair() {
        let (mut pipeline, _) = Pipeline::start("open tasks".into());
        let _ = pipeline.apply(Event::Planned(plan()));

        let mut fills = 0;
        for step in pipeline.apply(Event::Queried(Ok(json!([])))) {
            if matches!(step, Step::Fill { .. }) {
                fills += 1;
            }
        }
        for step in pipeline.apply(Event::Rendered("<p></p>".into())) {
            if matches!(step, Step::Fill { .. }) {
                fills += 1;
            }
        }
        assert_eq!(fills, 1);
    }
}
