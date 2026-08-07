DO $migration$
DECLARE
    row_record RECORD;
BEGIN
    FOR row_record IN
        SELECT ns.note_id, ns.user_id, ns.capabilities,
               CASE
                   WHEN n.notebook_id IS NULL THEN 'inbox'
                   ELSE 'matches_containing_notebook_share'
               END AS attribution
        FROM note_shares ns
        JOIN notes n ON n.id = ns.note_id
        WHERE n.notebook_id IS NULL
           OR EXISTS (
               SELECT 1
               FROM notebook_shares nbs
               WHERE nbs.notebook_id = n.notebook_id
                 AND nbs.user_id = ns.user_id
                 AND nbs.capabilities = ns.capabilities
           )
        ORDER BY ns.note_id, ns.user_id
    LOOP
        RAISE NOTICE 'unattributable note share: note_id=%, user_id=%, capabilities=%, kind=%',
            row_record.note_id,
            row_record.user_id,
            row_record.capabilities,
            row_record.attribution;
    END LOOP;
END
$migration$;
