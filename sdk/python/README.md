# fcp-sdk (Python)

The FCP admin API's client for Python 3.10+. It is generated from the API's
OpenAPI contract (`docs/openapi.json`), with a thin hand-written layer on
top.

The distribution holds two packages:
- `fcp_api` is generated: one module per operation under `fcp_api.api`, and
  the models under `fcp_api.models`.
- `fcp_sdk` is the layer:
  - a configured client (the token, the tenant, retries on 429 and 503);
  - the API's errors as `FcpError`;
  - list paging;
  - webhook verification.

```python
from fcp_api.api.users import users_create_user, users_list_users
from fcp_api.models import CreateUserRequest
from fcp_sdk import FcpError, client, ok, paginate, verify_webhook

fcp = client("https://admin.example.com", token="fcp_…")

ok(users_create_user.sync_detailed(
    client=fcp,
    body=CreateUserRequest(username="ada", password="…", role="end_user"),
))

for user in paginate(users_list_users.sync_detailed, client=fcp, limit=500):
    print(user.username)

try:
    ok(users_list_users.sync_detailed(client=client(url, "fcp_wrong")))
except FcpError as e:
    print(e.status, e.code, e.details, e.request_id)
```

Every operation also has `sync` (the parsed body only), `asyncio_detailed`
and `asyncio`. For asyncio, use `async_client(...)` and `apaginate`.

## How it behaves

- **Tokens.** An `fcp_…` token reaches only what its permissions name
  (docs/API_TOKENS.md).
- **Tenants.** `tenant_id=` picks the tenant for the static token or a
  platform admin's session.
- **Retries.** A 429 or a 503 is retried up to `retries` times (3 by
  default), waiting as `Retry-After` says, at most `max_retry_wait`
  seconds each.
- **Webhooks.** `verify_webhook(secret, request.headers["X-FCP-Signature"],
  raw_body)` is true only for a delivery signed with the endpoint's secret
  in the last five minutes. Pass the body as received, before parsing it.

## Developing it

- `uv run --extra test pytest` runs the tests.
- The live ones need a running admin, which `cargo test -p fcp-admin --test
  sdks` starts for them.
- When the API changes, regenerate `fcp_api` from the contract:

  ```sh
  uvx openapi-python-client@0.29.1 generate --path ../../docs/openapi.json \
    --meta none --output-path fcp_api --overwrite --config openapi-python-client.yaml
  ```

  CI fails if the committed `fcp_api` is not what the contract makes.
- The version's major and minor follow the FCP release. The patch is set
  when it is published.
