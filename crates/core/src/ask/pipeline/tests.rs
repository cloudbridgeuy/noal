//! The ask pipeline's tests, in their own file so `pipeline.rs` stays within
//! the workspace's line limit.

use super::{returned_column_names, stored_column_names, Event, Origin, Pipeline, Step};
use crate::ask::outcome::{Stage, Timing, Verdict};
use crate::ask::plan::{Column, ColumnKind, Parent, Plan};
use serde_json::json;

#[allow(clippy::unwrap_used)]
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

/// A plan whose shape has one extra column, so a template written for
/// [`plan`] does not fit it.
fn plan_with_a_different_shape() -> Plan {
    Plan {
        sql: "select id, title from ticket".into(),
        shape: vec![
            Column {
                name: "id".into(),
                kind: ColumnKind::Integer,
                description: String::new(),
                fields: Vec::new(),
            },
            Column {
                name: "title".into(),
                kind: ColumnKind::Text,
                description: String::new(),
                fields: Vec::new(),
            },
        ],
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
    let (_, steps) = Pipeline::start("open tasks".into(), None);
    assert!(matches!(steps.as_slice(), [Step::Plan { .. }]));
}

#[test]
fn a_plan_issues_a_query_and_a_render_together() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let steps = pipeline.apply(Event::Planned(plan()));
    assert!(matches!(
        steps.as_slice(),
        [Step::Query { .. }, Step::Render { .. }]
    ));
}

#[test]
fn happy_path_fills_once_the_query_answers_first() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
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
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
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
fn record_appends_timings_in_order() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    pipeline.record(Timing {
        stage: Stage::Plan,
        millis: 10,
    });
    pipeline.record(Timing {
        stage: Stage::Query,
        millis: 20,
    });

    // Spend both attempts: with a retry policy in place, one refusal is
    // no longer enough to reach `Done`.
    let _ = pipeline.apply(Event::Queried(Err("first refusal".into())));
    let done = pipeline.apply(Event::Queried(Err("second refusal".into())));
    let outcome = done_outcome(&done);
    assert_eq!(
        outcome.debug.timings,
        vec![
            Timing {
                stage: Stage::Plan,
                millis: 10
            },
            Timing {
                stage: Stage::Query,
                millis: 20
            },
        ]
    );
}

#[test]
fn a_refused_query_asks_the_model_to_plan_again() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));

    let steps = pipeline.apply(Event::Queried(Err("column \"nope\" does not exist".into())));
    match steps.as_slice() {
        [Step::Plan { prompt }] => {
            assert!(prompt.contains("# Previous attempts"));
            assert!(prompt.contains("column \"nope\" does not exist"));
        }
        other => panic!("expected exactly one Plan step, got {other:?}"),
    }

    // A late-arriving render for the refused pair must not turn into a
    // Fill: the rows it would pair with never arrived.
    let after_render = pipeline.apply(Event::Rendered("<p></p>".into()));
    assert!(after_render.is_empty());
}

#[test]
fn a_second_refused_query_ends_the_ask_instead_of_planning_a_third_time() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Err("first refusal".into())));
    // The model tries again; a fresh plan with no held template.
    let _ = pipeline.apply(Event::Planned(plan()));

    let done = pipeline.apply(Event::Queried(Err("second refusal".into())));
    match done.as_slice() {
        [Step::Done(outcome)] => {
            assert_eq!(
                outcome.verdict,
                Verdict::Failed {
                    stage: Stage::Query
                }
            );
            assert_eq!(outcome.debug.attempts.len(), 2);
            assert!(outcome
                .debug
                .attempts
                .iter()
                .all(|attempt| attempt.stage == Stage::Query));
        }
        other => panic!("expected exactly one Done step, not a third Plan, got {other:?}"),
    }
}

#[test]
fn a_refused_fill_asks_the_model_to_render_again() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("{{ rows.0.missing }}".into()));

    let steps = pipeline.apply(Event::Filled(Err("variable `missing` not found".into())));
    match steps.as_slice() {
        [Step::Render { prompt }] => {
            assert!(prompt.contains("# Previous attempts"));
            assert!(prompt.contains("variable `missing` not found"));
        }
        other => panic!("expected exactly one Render step, got {other:?}"),
    }
}

#[test]
fn a_second_refused_fill_ends_the_ask() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("{{ rows.0.missing }}".into()));
    let _ = pipeline.apply(Event::Filled(Err("first refusal".into())));
    let _ = pipeline.apply(Event::Rendered("{{ rows.0.also_missing }}".into()));

    let done = pipeline.apply(Event::Filled(Err("second refusal".into())));
    let outcome = done_outcome(&done);
    assert_eq!(outcome.verdict, Verdict::Failed { stage: Stage::Fill });
    assert_eq!(outcome.debug.attempts.len(), 2);
    assert!(outcome
        .debug
        .attempts
        .iter()
        .all(|attempt| attempt.stage == Stage::Fill));
}

#[test]
fn a_fill_that_carries_a_link_refuses_the_render_and_retries_it() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));

    let steps = pipeline.apply(Event::Filled(Ok("<a href=\"/x\">go</a>".into())));
    match steps.as_slice() {
        [Step::Render { prompt }] => {
            assert!(prompt.contains("# Previous attempts"));
            // The error names the token found in the OUTPUT, and the
            // artifact fed back is the TEMPLATE, never the filled HTML.
            assert!(prompt.contains("href"));
            assert!(prompt.contains("<p>{{ rows | length }}</p>"));
            assert!(!prompt.contains("<a href"));
        }
        other => panic!("expected exactly one Render retry, got {other:?}"),
    }
}

#[test]
fn a_tripped_fill_that_recovers_clears_the_latch_and_answers() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    let _ = pipeline.apply(Event::Filled(Ok("<a href=\"/x\">go</a>".into())));

    // The clean re-render must reach a Fill again: the latch the
    // tripped fill set has been cleared.
    let after_render = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    match after_render.as_slice() {
        [Step::Fill { .. }] => {}
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
fn a_second_tripped_fill_ends_the_ask_at_the_render_stage() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    let _ = pipeline.apply(Event::Filled(Ok("<a href=\"/x\">go</a>".into())));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));

    let done = pipeline.apply(Event::Filled(Ok("<script>x</script>".into())));
    let outcome = done_outcome(&done);
    assert_eq!(
        outcome.verdict,
        Verdict::Failed {
            stage: Stage::Render
        }
    );
    assert_eq!(outcome.debug.attempts.len(), 2);
    assert!(
        outcome
            .debug
            .attempts
            .iter()
            .all(|attempt| attempt.stage == Stage::Render),
        "both refusals are recorded against Render"
    );
    assert_eq!(
        outcome.debug.attempts[1].error,
        "rendered output carries a forbidden token: script"
    );
}

#[test]
fn a_mixed_refusal_budget_exhausts_at_the_render_stage() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("{{ rows.0.missing }}".into()));

    // One Fill-stage refusal (Tera could not bind the template).
    let retry = pipeline.apply(Event::Filled(Err("variable `missing` not found".into())));
    assert!(matches!(retry.as_slice(), [Step::Render { .. }]));

    // The clean re-render fills again, but its output carries a
    // forbidden token: one Render-stage refusal.
    let after_render = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    assert!(matches!(after_render.as_slice(), [Step::Fill { .. }]));

    // Both kinds share `render_attempts`, so the budget is spent even
    // though each stage was refused only once.
    let done = pipeline.apply(Event::Filled(Ok("<a href=\"/x\">go</a>".into())));
    let outcome = done_outcome(&done);
    assert_eq!(
        outcome.verdict,
        Verdict::Failed {
            stage: Stage::Render
        }
    );
    assert_eq!(outcome.debug.attempts.len(), 2);
    assert_eq!(outcome.debug.attempts[0].stage, Stage::Fill);
    assert_eq!(outcome.debug.attempts[1].stage, Stage::Render);
}

#[test]
fn debug_attempts_holds_every_refusal_in_order() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Err("bad sql".into())));
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    let _ = pipeline.apply(Event::Rendered("{{ rows.0.missing }}".into()));
    let _ = pipeline.apply(Event::Filled(Err("missing once".into())));

    let done = pipeline.apply(Event::Filled(Err("missing twice".into())));
    let outcome = done_outcome(&done);
    let stages: Vec<_> = outcome
        .debug
        .attempts
        .iter()
        .map(|attempt| (attempt.stage, attempt.error.as_str()))
        .collect();
    assert_eq!(
        stages,
        vec![
            (Stage::Query, "bad sql"),
            (Stage::Fill, "missing once"),
            (Stage::Fill, "missing twice"),
        ]
    );
}

#[test]
fn a_replan_with_an_equal_shape_keeps_the_template_and_only_queries() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    let _ = pipeline.apply(Event::Queried(Err("bad sql".into())));

    let steps = pipeline.apply(Event::Planned(plan()));
    assert!(
        matches!(steps.as_slice(), [Step::Query { .. }]),
        "expected only a Query step, got {steps:?}"
    );
}

#[test]
fn a_replan_with_a_different_shape_drops_the_template_and_renders_again() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    let _ = pipeline.apply(Event::Queried(Err("bad sql".into())));

    let steps = pipeline.apply(Event::Planned(plan_with_a_different_shape()));
    assert!(
        matches!(steps.as_slice(), [Step::Query { .. }, Step::Render { .. }]),
        "expected a Query and a Render step, got {steps:?}"
    );
}

#[test]
fn a_retry_that_keeps_the_template_still_reaches_answered() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Rendered("<p>{{ rows | length }}</p>".into()));
    let _ = pipeline.apply(Event::Queried(Err("bad sql".into())));
    // The re-plan has the same shape, so only a Query is issued and the
    // held template is kept for the fill.
    let requeue = pipeline.apply(Event::Planned(plan()));
    assert!(matches!(requeue.as_slice(), [Step::Query { .. }]));

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
fn the_fill_step_is_never_issued_twice_for_one_pair() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
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

fn stored_template() -> String {
    "<p>{{ rows | length }}</p>".into()
}

#[test]
fn reopening_issues_only_a_query_and_carries_the_stored_artifacts() {
    let (pipeline, steps) = Pipeline::reopen("open tasks".into(), plan(), stored_template());
    match steps.as_slice() {
        [Step::Query { sql }] => {
            assert_eq!(
                sql,
                "select coalesce(json_agg(t), '[]')::text as rows from (select id from ticket) t"
            );
        }
        other => panic!("expected exactly one Query step, got {other:?}"),
    }
    // The debug panel sees the stored plan and template, not blanks.
    assert!(pipeline.debug.plan.is_some());
    assert_eq!(
        pipeline.debug.template.as_deref(),
        Some("<p>{{ rows | length }}</p>")
    );
}

#[test]
fn a_reopened_ask_answers_without_a_single_model_call() {
    let (mut pipeline, steps) = Pipeline::reopen("open tasks".into(), plan(), stored_template());

    // The only steps a reopened ask may issue are Query and Fill; a Plan
    // or Render step would mean the model was called.
    assert!(matches!(steps.as_slice(), [Step::Query { .. }]));

    let after_query = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));
    match after_query.as_slice() {
        [Step::Fill {
            template,
            rows: filled_rows,
        }] => {
            assert_eq!(template, "<p>{{ rows | length }}</p>");
            assert_eq!(filled_rows, &json!([{ "id": 1 }]));
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
    assert_eq!(outcome.origin, Origin::Reopened);
    assert!(outcome.debug.attempts.is_empty());
}

#[test]
fn a_refused_query_on_a_reopened_ask_gives_up_instead_of_calling_the_model() {
    let (mut pipeline, _) = Pipeline::reopen("open tasks".into(), plan(), stored_template());

    let done = pipeline.apply(Event::Queried(Err("column \"nope\" does not exist".into())));
    let outcome = done_outcome(&done);
    assert_eq!(
        outcome.verdict,
        Verdict::Failed {
            stage: Stage::Query
        }
    );
    assert_eq!(outcome.origin, Origin::Reopened);
    // The refusal is still recorded, so the debug panel can say why.
    assert_eq!(outcome.debug.attempts.len(), 1);
}

#[test]
fn a_refused_fill_on_a_reopened_ask_gives_up_instead_of_calling_the_model() {
    let (mut pipeline, _) =
        Pipeline::reopen("open tasks".into(), plan(), "{{ rows.0.missing }}".into());
    let _ = pipeline.apply(Event::Queried(Ok(json!([{ "id": 1 }]))));

    let done = pipeline.apply(Event::Filled(Err("variable `missing` not found".into())));
    let outcome = done_outcome(&done);
    assert_eq!(outcome.verdict, Verdict::Failed { stage: Stage::Fill });
    assert_eq!(outcome.origin, Origin::Reopened);
    assert_eq!(outcome.debug.attempts.len(), 1);
}

#[test]
fn an_asked_ask_keeps_its_retry_policy_and_origin() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));

    // One refusal is not the end for an asked ask; the model plans again.
    let steps = pipeline.apply(Event::Queried(Err("bad sql".into())));
    assert!(matches!(steps.as_slice(), [Step::Plan { .. }]));

    let done = pipeline.apply(Event::Queried(Err("worse sql".into())));
    assert_eq!(done_outcome(&done).origin, Origin::Asked);
}

fn text_column(name: &str) -> Column {
    Column {
        name: name.to_owned(),
        kind: ColumnKind::Text,
        description: String::new(),
        fields: Vec::new(),
    }
}

/// A reopened pipeline whose stored plan promises a `name` column.
fn reopened_pipeline_with_name_shape() -> Pipeline {
    let drifted_plan = Plan {
        sql: "select name from t".into(),
        shape: vec![text_column("name")],
    };
    Pipeline::reopen(
        "open tasks".into(),
        drifted_plan,
        "<p>{{ row.name }}</p>".into(),
    )
    .0
}

#[test]
fn a_reopened_ask_whose_rows_drift_fail_at_fill_and_record_the_attempt() {
    // The stored plan promises `name`; the query came back with `nome` —
    // the table was renamed under the saved window.
    let mut pipeline = reopened_pipeline_with_name_shape();

    let done = pipeline.apply(Event::Queried(Ok(json!([{ "nome": "x" }]))));
    let outcome = done_outcome(&done);

    assert_eq!(outcome.verdict, Verdict::Failed { stage: Stage::Fill });
    assert_eq!(outcome.origin, Origin::Reopened);
    assert_eq!(pipeline.debug.attempts.len(), 1);
    assert_eq!(pipeline.debug.attempts[0].stage, Stage::Fill);
    assert_eq!(pipeline.debug.attempts[0].artifact, "select name from t");
    assert!(
        pipeline.debug.attempts[0].error.contains("nome"),
        "the diff names what the query returned"
    );
    assert!(
        pipeline.debug.attempts[0].error.contains("name"),
        "the diff names what the stored shape expects"
    );
    // The helpers behind the diff text are total over any rows.
    assert_eq!(
        returned_column_names(&json!([{ "nome": "x" }])),
        vec!["nome"]
    );
    assert_eq!(stored_column_names(&[text_column("name")]), vec!["name"]);
}

#[test]
fn a_reopened_ask_whose_rows_fit_the_stored_shape_fills_normally() {
    let mut pipeline = reopened_pipeline_with_name_shape();

    let after_query = pipeline.apply(Event::Queried(Ok(json!([{ "name": "x" }]))));
    match after_query.as_slice() {
        [Step::Fill { .. }] => {}
        other => panic!("expected exactly one Fill step, got {other:?}"),
    }

    let done = pipeline.apply(Event::Filled(Ok("<p>x</p>".into())));
    assert_eq!(
        done_outcome(&done).verdict,
        Verdict::Answered {
            html: "<p>x</p>".into()
        }
    );
    assert!(done_outcome(&done).debug.attempts.is_empty());
}

#[test]
fn an_asked_ask_is_not_shape_gated() {
    // The same drift on an asked ask keeps today's behaviour: the fill
    // proceeds (or retries as it does today); the check never fires.
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    let _ = pipeline.apply(Event::Rendered(stored_template()));

    let after_query = pipeline.apply(Event::Queried(Ok(json!([{ "nope": 1 }]))));
    match after_query.as_slice() {
        [Step::Fill { .. }] => {}
        other => panic!("expected exactly one Fill step, got {other:?}"),
    }
    assert!(pipeline.debug.attempts.is_empty());
}

#[test]
fn a_started_pipeline_holds_its_parent_for_both_prompts() {
    let parent = Parent {
        request: "open tickets".into(),
        plan: plan(),
        template: "<ul></ul>".into(),
    };
    let (mut pipeline, steps) = Pipeline::start("only the blockers".into(), Some(parent));
    match steps.as_slice() {
        [Step::Plan { prompt }] => {
            assert!(prompt.contains("# Previous window"));
            assert!(prompt.contains("select id from ticket"));
            // The new request stays last, after the parent's context.
            let previous = prompt.find("# Previous window").unwrap();
            let request = prompt.find("# Request").unwrap();
            assert!(previous < request);
        }
        other => panic!("expected exactly one Plan step, got {other:?}"),
    }

    // The held parent reaches the render prompt too, via on_planned.
    let steps = pipeline.apply(Event::Planned(plan()));
    match steps.as_slice() {
        [Step::Query { .. }, Step::Render { prompt }] | [Step::Render { prompt }] => {
            assert!(prompt.contains("# The previous window's template"));
            assert!(prompt.contains("<ul></ul>"));
        }
        other => panic!("unexpected steps {other:?}"),
    }
}

#[test]
fn a_root_ask_carries_no_previous_window() {
    let (mut pipeline, _) = Pipeline::start("open tasks".into(), None);
    let _ = pipeline.apply(Event::Planned(plan()));
    assert_eq!(pipeline.parent, None);
}

#[test]
fn a_reopened_pipeline_has_no_parent_context() {
    let (pipeline, _) = Pipeline::reopen("open tasks".into(), plan(), "<p></p>".into());
    assert_eq!(pipeline.parent, None);
}
