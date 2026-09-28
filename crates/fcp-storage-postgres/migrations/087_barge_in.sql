-- Enterprise calling, phase 11 (docs/ENTERPRISE_CALLING.md §3.11): a
-- shared line may let its phones barge into an active appearance — the
-- phone's INVITE naming the appearance joins the call, heard by both
-- parties.
ALTER TABLE line_bindings ADD COLUMN IF NOT EXISTS barge_in BOOLEAN NOT NULL DEFAULT false;
