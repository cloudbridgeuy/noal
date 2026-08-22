-- Bootstrap.
--
-- noal has no domain tables yet. This migration exists so the runner has
-- something to apply, and so the ledger proves the pipeline works end to end
-- before any real schema depends on it.
--
-- Add domain tables in later numbered files. Never edit an applied migration:
-- the ledger records that it ran, not what it said, so a change to an applied
-- file is a change that no environment will ever pick up.

-- A no-op that is safe to run against any database.
SELECT 1;
