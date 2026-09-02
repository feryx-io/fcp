-- Encryption at rest: envelope-wrapped data encryption keys (DEKs).
--
-- Secret columns (users.digest_password / sip_ha1_hash /
-- sip_provisioning_password, trunks.auth_password, conference_meetings
-- guest_pin / host_pin, webhook_endpoints.endpoint_json->>'secret') hold
-- AES-256-GCM ciphertext in the form `enc:v1:<dek id>:<base64>` once a
-- deployment enables encryption (FCP_ENCRYPTION_PROVIDER). Legacy plaintext
-- rows stay readable and are re-sealed lazily on write or in bulk via the
-- admin re-encrypt endpoint. This table stores the DEKs, each wrapped by the
-- deployment's key-encryption key (local master key or AWS KMS); the KEK
-- itself never touches the database.

CREATE TABLE IF NOT EXISTS data_encryption_keys (
    id TEXT PRIMARY KEY,
    kek_id TEXT NOT NULL,
    wrapped_key BYTEA NOT NULL,
    active BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Exactly one DEK seals new writes.
CREATE UNIQUE INDEX IF NOT EXISTS idx_data_encryption_keys_active
    ON data_encryption_keys (active) WHERE active;
