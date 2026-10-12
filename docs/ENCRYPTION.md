# Encryption at Rest

FCP seals the secrets it must be able to read back in plaintext, so a copy of
the database (backup, replica, leaked dump) does not hand an attacker working
SIP credentials, trunk passwords, conference PINs or webhook signing keys.
Values that are only ever *compared* (user login passwords, voicemail PINs,
API tokens, provisioning tokens) are already stored as one-way hashes and are
not part of this.

## Model

Envelope encryption with two key tiers:

```
KEK  (local master key, or an AWS KMS key)      never stored in the database
 └─wraps─▶ DEK  (AES-256, in data_encryption_keys, stored only wrapped)
            └─seals─▶ column values   enc:v1:<dek id>:<base64 nonce+ciphertext>
```

- **Cipher**: AES-256-GCM, random 96-bit nonce per value, DEK id bound as
  associated data (a ciphertext cannot be re-labelled to another key).
- **Sealed columns**: `users.digest_password`, `users.sip_ha1_hash`,
  `users.sip_provisioning_password`, `trunks.auth_password`,
  `conference_meetings.guest_pin` / `host_pin`, the `secret` and a rotation's
  `previous_secret` inside `webhook_endpoints.endpoint_json`, and each tenant's pseudonym key in
  `subject_pseudonym_keys.secret` (`docs/COMPLIANCE.md`: subject erasure). The list is `SEALED_COLUMNS` in
  `crates/fcp-storage-postgres/src/encryption.rs` and is reported by the
  status endpoint.
- **Self-describing values**: a column may hold legacy plaintext and
  ciphertext side by side. Reads pass plaintext through; writes always seal.
  Nothing has to be migrated before enabling encryption.
- **Key ring**: every DEK ever created stays in `data_encryption_keys` for
  reads; exactly one is `active` and seals new writes. Nodes that see a DEK
  they have not loaded (another node rotated) refresh the ring once before
  failing.
- **Fail closed**: a backend *without* encryption configured that reads a
  sealed value returns an error rather than a wrong credential. A master key
  that does not match the key the active DEK was wrapped with fails at
  startup with a clear message (the KEK id is a fingerprint of the key).

Code: `crates/fcp-crypto` (cipher, providers, settings),
`crates/fcp-storage-postgres/src/encryption.rs` (key ring, re-seal),
`crates/fcp-core/src/encryption.rs` (`EncryptionStore` contract).

## Enabling

Every binary that opens the database reads the same environment:

| Variable | Meaning |
|---|---|
| `FCP_ENCRYPTION_PROVIDER` | `none` (default), `local`, or `aws-kms` |
| `FCP_MASTER_KEY` | `local`: base64 of 32 random bytes (`openssl rand -base64 32`) |
| `FCP_MASTER_KEY_FILE` | `local`: path to a file containing that base64 string (mounted secret) |
| `FCP_AWS_KMS_KEY_ID` | `aws-kms`: key id, ARN or alias; credentials/region from the standard AWS chain |

Setting `FCP_MASTER_KEY` without a provider selects `local`. `aws-kms`
requires binaries built with the `aws-kms` cargo feature
(`fcp-storage-postgres/aws-kms`); it uses `GenerateDataKey` for new DEKs and
`Decrypt` to unwrap, so the KMS key never leaves AWS.

On first start with encryption enabled the backend creates the initial DEK.
From then on new writes are sealed. To seal what is already stored:

```
POST /api/v1/system/encryption/reencrypt      # platform admin
```

## Operating

| Endpoint (platform admin) | Effect |
|---|---|
| `GET /api/v1/system/encryption` | provider, KEK id, active DEK, all DEK ids, sealed columns |
| `POST /api/v1/system/encryption/rotate` | new DEK becomes active; old ones stay for reads |
| `POST /api/v1/system/encryption/reencrypt` | rewrite every plaintext or old-DEK value with the active DEK; returns counts per column |
| `POST /api/v1/system/encryption/rewrap` | move every DEK the previous KEK wraps to the current one; returns `rewrapped`, `already_current` and the ids of any key neither KEK here unwraps (`unreadable`) |
| `POST /api/v1/system/encryption/seal` | seal one value (`{"value": "…"}` → `{"sealed": "enc:v1:…"}`) for a credential that lives in a configuration file rather than a column — today the E911 validator's bearer token (`docs/E911.md`). The value is neither stored nor logged |

**Rotation** is two steps: rotate, then reencrypt once every node has picked
up the new ring (they do so lazily on the first unknown-DEK read, or at
restart). Old DEKs can be deleted from `data_encryption_keys` only after a
re-encrypt pass reports `already_current` for everything.

**Changing the master key / KMS key** is not a rotate: existing DEKs are
wrapped by the old KEK. A node also given the KEK it is moving from —
`FCP_PREVIOUS_MASTER_KEY` (or `FCP_PREVIOUS_MASTER_KEY_FILE`) for a local
key, `FCP_PREVIOUS_AWS_KMS_KEY_ID` for KMS — reads the keys either wraps.
The move:

1. Give every node the new key (`FCP_MASTER_KEY` or `FCP_AWS_KMS_KEY_ID`)
   and the old one as the previous, and restart them. A node with the new
   key alone refuses to start while the active DEK is wrapped by the old.
2. `POST /api/v1/system/encryption/rewrap` once: each old DEK is unwrapped
   with the previous KEK and wrapped with the new one. The values sealed
   with them are not touched — a DEK's material does not change, only what
   wraps it — so this is quick however many secrets there are. Its answer
   lists any key neither KEK unwraps.
3. Remove the previous key from every node's environment and restart.

The old key is needed until step 2 has run; keep it in the recovery plan
until then. With AWS KMS, KMS automatic key rotation keeps older key
versions decryptable behind the same key id and needs none of this; moving
to another KMS key, or between local and KMS, does.

**Backups**: the database alone is not enough to read secrets. Keep the master
key (or KMS key access) in your recovery plan.

## Not covered

- Media at rest (recordings, voicemail audio, TTS cache) — file-system or
  object-store encryption is the deployment's responsibility.
- Redis (registrations, presence, cluster state) — no long-lived secrets.
- Transport encryption (TLS/SRTP) is separate and already supported.
