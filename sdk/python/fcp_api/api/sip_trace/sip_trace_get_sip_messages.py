from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.sip_trace_list_response import SipTraceListResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    call_id: str,
    *,
    sip_call_id: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["sip_call_id"] = sip_call_id


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/calls/{call_id}/sip-messages".format(call_id=quote(str(call_id), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | SipTraceListResponse:
    if response.status_code == 200:
        response_200 = SipTraceListResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | SipTraceListResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    sip_call_id: str | Unset = UNSET,

) -> Response[ErrorResponse | SipTraceListResponse]:
    """ Get all SIP trace messages for a call, ordered by timestamp.

     GET /api/v1/calls/:call_id/sip-messages

    Query parameters:
    - sip_call_id: Alternative lookup by SIP Call-ID header value

    Args:
        call_id (str):
        sip_call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SipTraceListResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
sip_call_id=sip_call_id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    sip_call_id: str | Unset = UNSET,

) -> ErrorResponse | SipTraceListResponse | None:
    """ Get all SIP trace messages for a call, ordered by timestamp.

     GET /api/v1/calls/:call_id/sip-messages

    Query parameters:
    - sip_call_id: Alternative lookup by SIP Call-ID header value

    Args:
        call_id (str):
        sip_call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SipTraceListResponse
     """


    return sync_detailed(
        call_id=call_id,
client=client,
sip_call_id=sip_call_id,

    ).parsed

async def asyncio_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    sip_call_id: str | Unset = UNSET,

) -> Response[ErrorResponse | SipTraceListResponse]:
    """ Get all SIP trace messages for a call, ordered by timestamp.

     GET /api/v1/calls/:call_id/sip-messages

    Query parameters:
    - sip_call_id: Alternative lookup by SIP Call-ID header value

    Args:
        call_id (str):
        sip_call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SipTraceListResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
sip_call_id=sip_call_id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    sip_call_id: str | Unset = UNSET,

) -> ErrorResponse | SipTraceListResponse | None:
    """ Get all SIP trace messages for a call, ordered by timestamp.

     GET /api/v1/calls/:call_id/sip-messages

    Query parameters:
    - sip_call_id: Alternative lookup by SIP Call-ID header value

    Args:
        call_id (str):
        sip_call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SipTraceListResponse
     """


    return (await asyncio_detailed(
        call_id=call_id,
client=client,
sip_call_id=sip_call_id,

    )).parsed
