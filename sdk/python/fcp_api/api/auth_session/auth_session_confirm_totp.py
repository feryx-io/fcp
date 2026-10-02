from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.login_response import LoginResponse
from ...models.totp_confirm_request import TotpConfirmRequest
from typing import cast



def _get_kwargs(
    *,
    body: TotpConfirmRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/auth/totp/confirm",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | LoginResponse:
    if response.status_code == 200:
        response_200 = LoginResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | LoginResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: TotpConfirmRequest,

) -> Response[ErrorResponse | LoginResponse]:
    """ POST /api/v1/auth/totp/confirm — finish setting it up, and sign in.

    Args:
        body (TotpConfirmRequest): A code from the app, proving it holds the secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | LoginResponse]
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
    body: TotpConfirmRequest,

) -> ErrorResponse | LoginResponse | None:
    """ POST /api/v1/auth/totp/confirm — finish setting it up, and sign in.

    Args:
        body (TotpConfirmRequest): A code from the app, proving it holds the secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | LoginResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: TotpConfirmRequest,

) -> Response[ErrorResponse | LoginResponse]:
    """ POST /api/v1/auth/totp/confirm — finish setting it up, and sign in.

    Args:
        body (TotpConfirmRequest): A code from the app, proving it holds the secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | LoginResponse]
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
    body: TotpConfirmRequest,

) -> ErrorResponse | LoginResponse | None:
    """ POST /api/v1/auth/totp/confirm — finish setting it up, and sign in.

    Args:
        body (TotpConfirmRequest): A code from the app, proving it holds the secret.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | LoginResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
