"""A scoped API token (docs/DEVELOPER_GUIDE.md §2): made with your own
token, holding only what it names, and used by the integration it is for."""

import os

from fcp_api.api.api_tokens import api_tokens_create_api_token
from fcp_api.api.users import users_list_users
from fcp_api.models import CreateApiTokenRequest
from fcp_sdk import client, ok


def run(url: str, token: str) -> dict:
    fcp = client(url, token)

    # The secret is shown once: store it where the integration reads it.
    made = ok(
        api_tokens_create_api_token.sync_detailed(
            client=fcp,
            body=CreateApiTokenRequest(name="crm-sync", permissions=["user_read"]),
        )
    )

    crm = client(url, made.secret)
    users = ok(users_list_users.sync_detailed(client=crm, limit=50))
    return {"token": made.token.id, "users": [u.username for u in users.data]}


if __name__ == "__main__":
    print(run(os.environ["FCP_URL"], os.environ["FCP_TOKEN"]))
