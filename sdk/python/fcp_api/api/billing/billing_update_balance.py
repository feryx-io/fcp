from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.balance_response import BalanceResponse
from ...models.error_response import ErrorResponse
from ...models.update_balance_request import UpdateBalanceRequest
from typing import cast



def _get_kwargs(
    *,
    body: UpdateBalanceRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/billing/balance",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> BalanceResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = BalanceResponse.from_dict(response.json())



        return response_200

    if response.status_code == 201:
        response_201 = BalanceResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[BalanceResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: UpdateBalanceRequest,

) -> Response[BalanceResponse | ErrorResponse]:
    """ PUT /api/v1/billing/balance — Create or update balance settings.

     Creates the balance row (at zero) when the tenant has none. The amount
    is never changed here; use transactions.

    Args:
        body (UpdateBalanceRequest): Balance settings update. Every field is optional; the balance
            amount
            itself is only changed through transactions.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[BalanceResponse | ErrorResponse]
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
    body: UpdateBalanceRequest,

) -> BalanceResponse | ErrorResponse | None:
    """ PUT /api/v1/billing/balance — Create or update balance settings.

     Creates the balance row (at zero) when the tenant has none. The amount
    is never changed here; use transactions.

    Args:
        body (UpdateBalanceRequest): Balance settings update. Every field is optional; the balance
            amount
            itself is only changed through transactions.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        BalanceResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: UpdateBalanceRequest,

) -> Response[BalanceResponse | ErrorResponse]:
    """ PUT /api/v1/billing/balance — Create or update balance settings.

     Creates the balance row (at zero) when the tenant has none. The amount
    is never changed here; use transactions.

    Args:
        body (UpdateBalanceRequest): Balance settings update. Every field is optional; the balance
            amount
            itself is only changed through transactions.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[BalanceResponse | ErrorResponse]
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
    body: UpdateBalanceRequest,

) -> BalanceResponse | ErrorResponse | None:
    """ PUT /api/v1/billing/balance — Create or update balance settings.

     Creates the balance row (at zero) when the tenant has none. The amount
    is never changed here; use transactions.

    Args:
        body (UpdateBalanceRequest): Balance settings update. Every field is optional; the balance
            amount
            itself is only changed through transactions.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        BalanceResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
