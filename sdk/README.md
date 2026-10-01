# SDKs

Clients for the FCP admin API, generated from its OpenAPI contract
(`docs/openapi.json`, served at `/api/v1/openapi.json`), each with a thin
hand-written layer: the token and tenant, retries on 429 and 503, the API's
errors, list paging, and webhook verification.

| | Package | Generated from the contract by |
|---|---|---|
| [`typescript/`](typescript/README.md) | `@fcp/sdk` | `openapi-typescript` + `openapi-fetch` |
| [`python/`](python/README.md) | `fcp-sdk` (`fcp_sdk`, `fcp_api`) | `openapi-python-client` |

CI checks that the generated code is what the contract makes (the **SDKs**
job), and runs each SDK's own tests against a live admin
(`crates/fcp-admin/tests/sdks.rs`).

Each SDK's `examples/` holds the developer guide's examples
(`docs/DEVELOPER_GUIDE.md`): a scoped token, a webhook receiver, a flow that
listens, and a live call held, resumed and ended. CI runs them against the
admin and a call manager with a call up, and fails if the guide quotes one
differently from its file. The versions' major and minor follow the
FCP release; publishing to npm and PyPI is done by hand, with the patch set
then.
