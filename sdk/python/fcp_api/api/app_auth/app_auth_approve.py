from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.approve_request import ApproveRequest
from ...models.approve_response import ApproveResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    *,
    body: ApproveRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/auth/app/approve",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ApproveResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ApproveResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ApproveResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient,
    body: ApproveRequest,

) -> Response[ApproveResponse | ErrorResponse]:
    """ POST /api/v1/auth/app/approve — the signed-in user grants the app's
    sign-in: the address to send the browser to, with a one-time code.

    Args:
        body (ApproveRequest): What `POST /auth/app/approve` takes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApproveResponse | ErrorResponse]
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
    client: AuthenticatedClient,
    body: ApproveRequest,

) -> ApproveResponse | ErrorResponse | None:
    """ POST /api/v1/auth/app/approve — the signed-in user grants the app's
    sign-in: the address to send the browser to, with a one-time code.

    Args:
        body (ApproveRequest): What `POST /auth/app/approve` takes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApproveResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient,
    body: ApproveRequest,

) -> Response[ApproveResponse | ErrorResponse]:
    """ POST /api/v1/auth/app/approve — the signed-in user grants the app's
    sign-in: the address to send the browser to, with a one-time code.

    Args:
        body (ApproveRequest): What `POST /auth/app/approve` takes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApproveResponse | ErrorResponse]
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
    client: AuthenticatedClient,
    body: ApproveRequest,

) -> ApproveResponse | ErrorResponse | None:
    """ POST /api/v1/auth/app/approve — the signed-in user grants the app's
    sign-in: the address to send the browser to, with a one-time code.

    Args:
        body (ApproveRequest): What `POST /auth/app/approve` takes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApproveResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
