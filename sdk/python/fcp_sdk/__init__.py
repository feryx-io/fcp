"""The FCP admin API's Python client (docs/DEVELOPER_SURFACE.md §3, phase 4).

A client generated from the OpenAPI contract (the ``fcp_api`` package: ``fcp_api.api``, one
module per operation, and ``fcp_api.models``), with what the generator leaves out:
authentication and the tenant header, retries on 429 and 503, the API's
errors as ``FcpError``, list paging, and webhook verification::

    from fcp_sdk import client, ok, paginate
    from fcp_api.api.users import users_list_users, users_get_user

    fcp = client("https://admin.example.com", token="fcp_…")
    alice = ok(users_get_user.sync_detailed(client=fcp, username="alice"))
    for user in paginate(users_list_users.sync_detailed, client=fcp, limit=500):
        print(user.username)
"""

from fcp_api import api, models
from fcp_api.client import AuthenticatedClient
from fcp_api.types import UNSET, Response
from .client import async_client, client
from .errors import FcpError, ok
from .paginate import apaginate, paginate
from .webhook import verify_webhook

__all__ = [
    "AuthenticatedClient",
    "FcpError",
    "Response",
    "UNSET",
    "api",
    "apaginate",
    "async_client",
    "client",
    "models",
    "ok",
    "paginate",
    "verify_webhook",
]
