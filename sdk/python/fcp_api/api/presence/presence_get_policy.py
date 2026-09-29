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

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/presence/{aor}/policy".format(aor=quote(str(aor), safe=""),),
    }


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

) -> Response[ErrorResponse | PresenceAuthPolicy]:
    """ GET /presence/:aor/policy — Get authorization policy for a presentity.

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceAuthPolicy]
     """


    kwargs = _get_kwargs(
        aor=aor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | PresenceAuthPolicy | None:
    """ GET /presence/:aor/policy — Get authorization policy for a presentity.

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceAuthPolicy
     """


    return sync_detailed(
        aor=aor,
client=client,

    ).parsed

async def asyncio_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | PresenceAuthPolicy]:
    """ GET /presence/:aor/policy — Get authorization policy for a presentity.

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceAuthPolicy]
     """


    kwargs = _get_kwargs(
        aor=aor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | PresenceAuthPolicy | None:
    """ GET /presence/:aor/policy — Get authorization policy for a presentity.

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceAuthPolicy
     """


    return (await asyncio_detailed(
        aor=aor,
client=client,

    )).parsed
