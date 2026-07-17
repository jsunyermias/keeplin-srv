-- Helper for the history access-window fix (HISTORY_VISIBILITY=access loophole).
--
-- `entity_history` now windows a collaborator's view by the *payload's own* causal
-- timestamp (`updated_at` inside the Change snapshot, `deleted_at` for tombstones)
-- instead of the journal row's `received_at` — journal re-delivery (a reinstalled
-- client re-pushing from epoch) mints fresh `received_at` values and defeated the
-- old filter, leaking pre-access versions.
--
-- Those timestamps live inside client-supplied opaque JSON, so a bare
-- `::timestamptz` cast in the query would let one malformed payload turn every
-- history read into a 500. This function is the safe cast: NULL on anything
-- unparseable (the query then falls back to `received_at`, matching how the
-- displayed version timestamp has always been derived).
CREATE OR REPLACE FUNCTION keeplin_try_timestamptz(value text)
RETURNS timestamptz
LANGUAGE plpgsql
STABLE
PARALLEL SAFE
AS $$
BEGIN
    RETURN value::timestamptz;
EXCEPTION WHEN OTHERS THEN
    RETURN NULL;
END;
$$;
