-- Video in the softphone (docs/SOFTPHONE_VIDEO.md §2.2, §2.7): a tenant's
-- word on video — whether calls carry it, what a phone's camera may send,
-- whether its people may share a screen in rooms they don't host. Empty
-- is every default: video on.
ALTER TABLE tenants
    ADD COLUMN IF NOT EXISTS video_json JSONB NOT NULL DEFAULT '{}'::jsonb;
