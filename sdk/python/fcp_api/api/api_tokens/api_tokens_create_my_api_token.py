from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.create_api_token_response import CreateApiTokenResponse
from ...models.create_personal_token_request import CreatePersonalTokenRequest
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    *,
    body: CreatePersonalTokenRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/api-tokens",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> CreateApiTokenResponse | ErrorResponse:
    if response.status_code == 201:
        response_201 = CreateApiTokenResponse.from_dict(response.json())



        return response_201

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
    *,
    client: AuthenticatedClient | Client,
    body: CreatePersonalTokenRequest,

) -> Response[CreateApiTokenResponse | ErrorResponse]:
    """ Make a personal token: the caller's own `/me` routes, nothing else.

     POST /api/v1/me/api-tokens

    Args:
        body (CreatePersonalTokenRequest): Request body for creating a personal API token.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CreateApiTokenResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: CreatePersonalTokenRequest,

) -> CreateApiTokenResponse | ErrorResponse | None:
    """ Make a personal token: the caller's own `/me` routes, nothing else.

     POST /api/v1/me/api-tokens

    Args:
        body (CreatePersonalTokenRequest): Request body for creating a personal API token.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CreateApiTokenResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: CreatePersonalTokenRequest,

) -> Response[CreateApiTokenResponse | ErrorResponse]:
    """ Make a personal token: the caller's own `/me` routes, nothing else.

     POST /api/v1/me/api-tokens

    Args:
        body (CreatePersonalTokenRequest): Request body for creating a personal API token.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CreateApiTokenResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: CreatePersonalTokenRequest,

) -> CreateApiTokenResponse | ErrorResponse | None:
    """ Make a personal token: the caller's own `/me` routes, nothing else.

     POST /api/v1/me/api-tokens

    Args:
        body (CreatePersonalTokenRequest): Request body for creating a personal API token.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CreateApiTokenResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
