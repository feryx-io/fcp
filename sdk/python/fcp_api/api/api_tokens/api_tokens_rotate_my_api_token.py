from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.create_api_token_response import CreateApiTokenResponse
from ...models.error_response import ErrorResponse
from ...models.rotate_api_token_request import RotateApiTokenRequest
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: RotateApiTokenRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/api-tokens/{id}/rotate".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> CreateApiTokenResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = CreateApiTokenResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[CreateApiTokenResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RotateApiTokenRequest,

) -> Response[CreateApiTokenResponse | ErrorResponse]:
    """ Rotate one of the caller's personal tokens.

     POST /api/v1/me/api-tokens/:id/rotate

    Args:
        id (str):
        body (RotateApiTokenRequest): Request body for rotating a token's secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CreateApiTokenResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RotateApiTokenRequest,

) -> CreateApiTokenResponse | ErrorResponse | None:
    """ Rotate one of the caller's personal tokens.

     POST /api/v1/me/api-tokens/:id/rotate

    Args:
        id (str):
        body (RotateApiTokenRequest): Request body for rotating a token's secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CreateApiTokenResponse | ErrorResponse
     """


    return sync_detailed(
        id=id,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RotateApiTokenRequest,

) -> Response[CreateApiTokenResponse | ErrorResponse]:
    """ Rotate one of the caller's personal tokens.

     POST /api/v1/me/api-tokens/:id/rotate

    Args:
        id (str):
        body (RotateApiTokenRequest): Request body for rotating a token's secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CreateApiTokenResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RotateApiTokenRequest,

) -> CreateApiTokenResponse | ErrorResponse | None:
    """ Rotate one of the caller's personal tokens.

     POST /api/v1/me/api-tokens/:id/rotate

    Args:
        id (str):
        body (RotateApiTokenRequest): Request body for rotating a token's secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CreateApiTokenResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
