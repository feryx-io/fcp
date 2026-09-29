from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.update_password_request import UpdatePasswordRequest
from typing import cast



def _get_kwargs(
    username: str,
    *,
    body: UpdatePasswordRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/users/{username}/password".format(username=quote(str(username), safe=""),),
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
    body: UpdatePasswordRequest,

) -> Response[Any | ErrorResponse]:
    """ Update a user's password.

     PUT /api/v1/users/:username/password

    Args:
        username (str):
        body (UpdatePasswordRequest): Update password request.

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
    body: UpdatePasswordRequest,

) -> Any | ErrorResponse | None:
    """ Update a user's password.

     PUT /api/v1/users/:username/password

    Args:
        username (str):
        body (UpdatePasswordRequest): Update password request.

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
    body: UpdatePasswordRequest,

) -> Response[Any | ErrorResponse]:
    """ Update a user's password.

     PUT /api/v1/users/:username/password

    Args:
        username (str):
        body (UpdatePasswordRequest): Update password request.

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
    body: UpdatePasswordRequest,

) -> Any | ErrorResponse | None:
    """ Update a user's password.

     PUT /api/v1/users/:username/password

    Args:
        username (str):
        body (UpdatePasswordRequest): Update password request.

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
