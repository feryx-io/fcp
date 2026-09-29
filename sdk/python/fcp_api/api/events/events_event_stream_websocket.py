from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    tenant_id: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    event_types: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["tenant_id"] = tenant_id

    params["call_id"] = call_id

    params["event_types"] = event_types


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/events",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 101:
        response_101 = cast(Any, None)
        return response_101

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
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    event_types: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ WebSocket handler for event streaming.

     GET /api/v1/events

    Query parameters:
    - tenant_id: Filter events for a specific tenant (UUID)
    - call_id: Filter events for a specific call (UUID)
    - event_types: Comma-separated list of event types (call,media,ivr,recording,system)

    Example:
    ws://localhost:8080/api/v1/events?event_types=call&tenant_id=123e4567-e89b-12d3-a456-426614174000

    Args:
        tenant_id (str | Unset):
        call_id (str | Unset):
        event_types (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        tenant_id=tenant_id,
call_id=call_id,
event_types=event_types,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    event_types: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ WebSocket handler for event streaming.

     GET /api/v1/events

    Query parameters:
    - tenant_id: Filter events for a specific tenant (UUID)
    - call_id: Filter events for a specific call (UUID)
    - event_types: Comma-separated list of event types (call,media,ivr,recording,system)

    Example:
    ws://localhost:8080/api/v1/events?event_types=call&tenant_id=123e4567-e89b-12d3-a456-426614174000

    Args:
        tenant_id (str | Unset):
        call_id (str | Unset):
        event_types (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        client=client,
tenant_id=tenant_id,
call_id=call_id,
event_types=event_types,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    event_types: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ WebSocket handler for event streaming.

     GET /api/v1/events

    Query parameters:
    - tenant_id: Filter events for a specific tenant (UUID)
    - call_id: Filter events for a specific call (UUID)
    - event_types: Comma-separated list of event types (call,media,ivr,recording,system)

    Example:
    ws://localhost:8080/api/v1/events?event_types=call&tenant_id=123e4567-e89b-12d3-a456-426614174000

    Args:
        tenant_id (str | Unset):
        call_id (str | Unset):
        event_types (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        tenant_id=tenant_id,
call_id=call_id,
event_types=event_types,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    event_types: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ WebSocket handler for event streaming.

     GET /api/v1/events

    Query parameters:
    - tenant_id: Filter events for a specific tenant (UUID)
    - call_id: Filter events for a specific call (UUID)
    - event_types: Comma-separated list of event types (call,media,ivr,recording,system)

    Example:
    ws://localhost:8080/api/v1/events?event_types=call&tenant_id=123e4567-e89b-12d3-a456-426614174000

    Args:
        tenant_id (str | Unset):
        call_id (str | Unset):
        event_types (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
tenant_id=tenant_id,
call_id=call_id,
event_types=event_types,

    )).parsed
