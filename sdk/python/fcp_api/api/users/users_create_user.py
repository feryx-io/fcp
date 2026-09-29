from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.create_user_request import CreateUserRequest
from ...models.error_response import ErrorResponse
from ...models.user_response import UserResponse
from typing import cast



def _get_kwargs(
    *,
    body: CreateUserRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/users",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | UserResponse:
    if response.status_code == 201:
        response_201 = UserResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | UserResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: CreateUserRequest,

) -> Response[ErrorResponse | UserResponse]:
    """ Create a new user.

     POST /api/v1/users

    **Security Note**: This endpoint uses constant-time operations to prevent
    user enumeration attacks. Even if a user exists, we perform password hashing
    before returning an error to match the timing of successful creation.

    Args:
        body (CreateUserRequest): Create user request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | UserResponse]
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
    body: CreateUserRequest,

) -> ErrorResponse | UserResponse | None:
    """ Create a new user.

     POST /api/v1/users

    **Security Note**: This endpoint uses constant-time operations to prevent
    user enumeration attacks. Even if a user exists, we perform password hashing
    before returning an error to match the timing of successful creation.

    Args:
        body (CreateUserRequest): Create user request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | UserResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: CreateUserRequest,

) -> Response[ErrorResponse | UserResponse]:
    """ Create a new user.

     POST /api/v1/users

    **Security Note**: This endpoint uses constant-time operations to prevent
    user enumeration attacks. Even if a user exists, we perform password hashing
    before returning an error to match the timing of successful creation.

    Args:
        body (CreateUserRequest): Create user request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | UserResponse]
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
    body: CreateUserRequest,

) -> ErrorResponse | UserResponse | None:
    """ Create a new user.

     POST /api/v1/users

    **Security Note**: This endpoint uses constant-time operations to prevent
    user enumeration attacks. Even if a user exists, we perform password hashing
    before returning an error to match the timing of successful creation.

    Args:
        body (CreateUserRequest): Create user request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | UserResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
