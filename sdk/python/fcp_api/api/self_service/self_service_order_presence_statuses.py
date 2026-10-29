from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.order_statuses_request import OrderStatusesRequest
from ...models.page_own_status_response import PageOwnStatusResponse
from typing import cast



def _get_kwargs(
    *,
    body: OrderStatusesRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/me/presence/statuses/order",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageOwnStatusResponse:
    if response.status_code == 200:
        response_200 = PageOwnStatusResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageOwnStatusResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: OrderStatusesRequest,

) -> Response[ErrorResponse | PageOwnStatusResponse]:
    """ PUT /me/presence/statuses/order

    Args:
        body (OrderStatusesRequest): A user's own statuses in a new order: every id, once.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageOwnStatusResponse]
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
    body: OrderStatusesRequest,

) -> ErrorResponse | PageOwnStatusResponse | None:
    """ PUT /me/presence/statuses/order

    Args:
        body (OrderStatusesRequest): A user's own statuses in a new order: every id, once.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageOwnStatusResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: OrderStatusesRequest,

) -> Response[ErrorResponse | PageOwnStatusResponse]:
    """ PUT /me/presence/statuses/order

    Args:
        body (OrderStatusesRequest): A user's own statuses in a new order: every id, once.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageOwnStatusResponse]
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
    body: OrderStatusesRequest,

) -> ErrorResponse | PageOwnStatusResponse | None:
    """ PUT /me/presence/statuses/order

    Args:
        body (OrderStatusesRequest): A user's own statuses in a new order: every id, once.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageOwnStatusResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
