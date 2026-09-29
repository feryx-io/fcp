from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_flow_trace_response import PageFlowTraceResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    call_id: str | Unset = UNSET,
    flow_id: str | Unset = UNSET,
    event_type: str | Unset = UNSET,
    time_from: str | Unset = UNSET,
    time_to: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["call_id"] = call_id

    params["flow_id"] = flow_id

    params["event_type"] = event_type

    params["time_from"] = time_from

    params["time_to"] = time_to

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/flow-traces",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageFlowTraceResponse:
    if response.status_code == 200:
        response_200 = PageFlowTraceResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageFlowTraceResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,
    flow_id: str | Unset = UNSET,
    event_type: str | Unset = UNSET,
    time_from: str | Unset = UNSET,
    time_to: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageFlowTraceResponse]:
    """ List flow execution traces with filters and pagination.

     GET /api/v1/flow-traces

    Args:
        call_id (str | Unset):
        flow_id (str | Unset):
        event_type (str | Unset):
        time_from (str | Unset):
        time_to (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageFlowTraceResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
flow_id=flow_id,
event_type=event_type,
time_from=time_from,
time_to=time_to,
limit=limit,
cursor=cursor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,
    flow_id: str | Unset = UNSET,
    event_type: str | Unset = UNSET,
    time_from: str | Unset = UNSET,
    time_to: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageFlowTraceResponse | None:
    """ List flow execution traces with filters and pagination.

     GET /api/v1/flow-traces

    Args:
        call_id (str | Unset):
        flow_id (str | Unset):
        event_type (str | Unset):
        time_from (str | Unset):
        time_to (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageFlowTraceResponse
     """


    return sync_detailed(
        client=client,
call_id=call_id,
flow_id=flow_id,
event_type=event_type,
time_from=time_from,
time_to=time_to,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,
    flow_id: str | Unset = UNSET,
    event_type: str | Unset = UNSET,
    time_from: str | Unset = UNSET,
    time_to: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageFlowTraceResponse]:
    """ List flow execution traces with filters and pagination.

     GET /api/v1/flow-traces

    Args:
        call_id (str | Unset):
        flow_id (str | Unset):
        event_type (str | Unset):
        time_from (str | Unset):
        time_to (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageFlowTraceResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
flow_id=flow_id,
event_type=event_type,
time_from=time_from,
time_to=time_to,
limit=limit,
cursor=cursor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,
    flow_id: str | Unset = UNSET,
    event_type: str | Unset = UNSET,
    time_from: str | Unset = UNSET,
    time_to: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageFlowTraceResponse | None:
    """ List flow execution traces with filters and pagination.

     GET /api/v1/flow-traces

    Args:
        call_id (str | Unset):
        flow_id (str | Unset):
        event_type (str | Unset):
        time_from (str | Unset):
        time_to (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageFlowTraceResponse
     """


    return (await asyncio_detailed(
        client=client,
call_id=call_id,
flow_id=flow_id,
event_type=event_type,
time_from=time_from,
time_to=time_to,
limit=limit,
cursor=cursor,

    )).parsed
