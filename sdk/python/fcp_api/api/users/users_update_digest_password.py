from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.update_digest_password_request import UpdateDigestPasswordRequest
from typing import cast



def _get_kwargs(
    username: str,
    *,
    body: UpdateDigestPasswordRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/users/{username}/digest-password".format(username=quote(str(username), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 204:
        response_204 = cast(Any, None)
        return response_204

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[Any | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    username: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateDigestPasswordRequest,

) -> Response[Any | ErrorResponse]:
    """ Update digest password (Cisco-style separate SIP credentials).

     PUT /api/v1/users/:username/digest-password

    This sets a separate password for SIP authentication, providing security isolation
    between API/web access and SIP phone registration (Cisco CUCM style).

    **Security Benefits:**
    - Compromised SIP credentials don't expose API/admin access
    - Different password policies for phones vs admin users
    - Separate rotation schedules for SIP and API credentials

    **Note:** If digest_password is not set (NULL), the main password is used for SIP.

    Args:
        username (str):
        body (UpdateDigestPasswordRequest): Update digest password request (for Cisco-style
            separate SIP credentials).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        username=username,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    username: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateDigestPasswordRequest,

) -> Any | ErrorResponse | None:
    """ Update digest password (Cisco-style separate SIP credentials).

     PUT /api/v1/users/:username/digest-password

    This sets a separate password for SIP authentication, providing security isolation
    between API/web access and SIP phone registration (Cisco CUCM style).

    **Security Benefits:**
    - Compromised SIP credentials don't expose API/admin access
    - Different password policies for phones vs admin users
    - Separate rotation schedules for SIP and API credentials

    **Note:** If digest_password is not set (NULL), the main password is used for SIP.

    Args:
        username (str):
        body (UpdateDigestPasswordRequest): Update digest password request (for Cisco-style
            separate SIP credentials).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        username=username,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    username: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateDigestPasswordRequest,

) -> Response[Any | ErrorResponse]:
    """ Update digest password (Cisco-style separate SIP credentials).

     PUT /api/v1/users/:username/digest-password

    This sets a separate password for SIP authentication, providing security isolation
    between API/web access and SIP phone registration (Cisco CUCM style).

    **Security Benefits:**
    - Compromised SIP credentials don't expose API/admin access
    - Different password policies for phones vs admin users
    - Separate rotation schedules for SIP and API credentials

    **Note:** If digest_password is not set (NULL), the main password is used for SIP.

    Args:
        username (str):
        body (UpdateDigestPasswordRequest): Update digest password request (for Cisco-style
            separate SIP credentials).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        username=username,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    username: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateDigestPasswordRequest,

) -> Any | ErrorResponse | None:
    """ Update digest password (Cisco-style separate SIP credentials).

     PUT /api/v1/users/:username/digest-password

    This sets a separate password for SIP authentication, providing security isolation
    between API/web access and SIP phone registration (Cisco CUCM style).

    **Security Benefits:**
    - Compromised SIP credentials don't expose API/admin access
    - Different password policies for phones vs admin users
    - Separate rotation schedules for SIP and API credentials

    **Note:** If digest_password is not set (NULL), the main password is used for SIP.

    Args:
        username (str):
        body (UpdateDigestPasswordRequest): Update digest password request (for Cisco-style
            separate SIP credentials).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        username=username,
client=client,
body=body,

    )).parsed
