from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.presence_auth_policy import PresenceAuthPolicy
from typing import cast



def _get_kwargs(
    aor: str,
    *,
    body: PresenceAuthPolicy,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/presence/{aor}/policy".format(aor=quote(str(aor), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PresenceAuthPolicy:
    if response.status_code == 200:
        response_200 = PresenceAuthPolicy.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PresenceAuthPolicy]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: PresenceAuthPolicy,

) -> Response[ErrorResponse | PresenceAuthPolicy]:
    """ PUT /presence/:aor/policy — Set authorization policy for a presentity.

    Args:
        aor (str):
        body (PresenceAuthPolicy): Authorization policy for presence subscriptions.

            Determines which watchers are allowed to subscribe to a presentity's state.
            If no policy is set for a presentity, all watchers are allowed (default open).

            Evaluation order (first match wins):
            1. Block list (AOR exact match) → deny
            2. Per-event block list → deny
            3. Block patterns (domain wildcard) → deny
            4. Allow list (AOR exact match) → allow
            5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
            6. Allow patterns (domain wildcard) → allow
            7. Time window check → deny if outside windows
            8. Default

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceAuthPolicy]
     """


    kwargs = _get_kwargs(
        aor=aor,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: PresenceAuthPolicy,

) -> ErrorResponse | PresenceAuthPolicy | None:
    """ PUT /presence/:aor/policy — Set authorization policy for a presentity.

    Args:
        aor (str):
        body (PresenceAuthPolicy): Authorization policy for presence subscriptions.

            Determines which watchers are allowed to subscribe to a presentity's state.
            If no policy is set for a presentity, all watchers are allowed (default open).

            Evaluation order (first match wins):
            1. Block list (AOR exact match) → deny
            2. Per-event block list → deny
            3. Block patterns (domain wildcard) → deny
            4. Allow list (AOR exact match) → allow
            5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
            6. Allow patterns (domain wildcard) → allow
            7. Time window check → deny if outside windows
            8. Default

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceAuthPolicy
     """


    return sync_detailed(
        aor=aor,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: PresenceAuthPolicy,

) -> Response[ErrorResponse | PresenceAuthPolicy]:
    """ PUT /presence/:aor/policy — Set authorization policy for a presentity.

    Args:
        aor (str):
        body (PresenceAuthPolicy): Authorization policy for presence subscriptions.

            Determines which watchers are allowed to subscribe to a presentity's state.
            If no policy is set for a presentity, all watchers are allowed (default open).

            Evaluation order (first match wins):
            1. Block list (AOR exact match) → deny
            2. Per-event block list → deny
            3. Block patterns (domain wildcard) → deny
            4. Allow list (AOR exact match) → allow
            5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
            6. Allow patterns (domain wildcard) → allow
            7. Time window check → deny if outside windows
            8. Default

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceAuthPolicy]
     """


    kwargs = _get_kwargs(
        aor=aor,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: PresenceAuthPolicy,

) -> ErrorResponse | PresenceAuthPolicy | None:
    """ PUT /presence/:aor/policy — Set authorization policy for a presentity.

    Args:
        aor (str):
        body (PresenceAuthPolicy): Authorization policy for presence subscriptions.

            Determines which watchers are allowed to subscribe to a presentity's state.
            If no policy is set for a presentity, all watchers are allowed (default open).

            Evaluation order (first match wins):
            1. Block list (AOR exact match) → deny
            2. Per-event block list → deny
            3. Block patterns (domain wildcard) → deny
            4. Allow list (AOR exact match) → allow
            5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
            6. Allow patterns (domain wildcard) → allow
            7. Time window check → deny if outside windows
            8. Default

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceAuthPolicy
     """


    return (await asyncio_detailed(
        aor=aor,
client=client,
body=body,

    )).parsed
