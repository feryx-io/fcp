-- Webinars W6.3a (docs/WEBINARS.md §2.8): a webinar's stream is kept as a
-- conference recording of kind `stream`, whose path is the stream's
-- directory. The orphan sweep asks whether a file lies under one; the
-- partial index keeps that to the streams.
CREATE INDEX IF NOT EXISTS idx_conference_recordings_streams
    ON conference_recordings (path) WHERE kind = 'stream';
