from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_log_line import PageLogLine
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    service: str | Unset = UNSET,
    kind: str | Unset = UNSET,
    level: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    q: str | Unset = UNSET,
    since: str | Unset = UNSET,
    until: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["service"] = service

    params["kind"] = kind

    params["level"] = level

    params["tenant"] = tenant

    params["call_id"] = call_id

    params["q"] = q

    params["since"] = since

    params["until"] = until

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/logs",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageLogLine:
    if response.status_code == 200:
        response_200 = PageLogLine.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageLogLine]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    service: str | Unset = UNSET,
    kind: str | Unset = UNSET,
    level: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    q: str | Unset = UNSET,
    since: str | Unset = UNSET,
    until: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageLogLine]:
    """ GET /logs

    Args:
        service (str | Unset):
        kind (str | Unset):
        level (str | Unset):
        tenant (str | Unset):
        call_id (str | Unset):
        q (str | Unset):
        since (str | Unset):
        until (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageLogLine]
     """


    kwargs = _get_kwargs(
        service=service,
kind=kind,
level=level,
tenant=tenant,
call_id=call_id,
q=q,
since=since,
until=until,
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
    service: str | Unset = UNSET,
    kind: str | Unset = UNSET,
    level: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    q: str | Unset = UNSET,
    since: str | Unset = UNSET,
    until: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageLogLine | None:
    """ GET /logs

    Args:
        service (str | Unset):
        kind (str | Unset):
        level (str | Unset):
        tenant (str | Unset):
        call_id (str | Unset):
        q (str | Unset):
        since (str | Unset):
        until (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageLogLine
     """


    return sync_detailed(
        client=client,
service=service,
kind=kind,
level=level,
tenant=tenant,
call_id=call_id,
q=q,
since=since,
until=until,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    service: str | Unset = UNSET,
    kind: str | Unset = UNSET,
    level: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    q: str | Unset = UNSET,
    since: str | Unset = UNSET,
    until: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageLogLine]:
    """ GET /logs

    Args:
        service (str | Unset):
        kind (str | Unset):
        level (str | Unset):
        tenant (str | Unset):
        call_id (str | Unset):
        q (str | Unset):
        since (str | Unset):
        until (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageLogLine]
     """


    kwargs = _get_kwargs(
        service=service,
kind=kind,
level=level,
tenant=tenant,
call_id=call_id,
q=q,
since=since,
until=until,
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
    service: str | Unset = UNSET,
    kind: str | Unset = UNSET,
    level: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    call_id: str | Unset = UNSET,
    q: str | Unset = UNSET,
    since: str | Unset = UNSET,
    until: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageLogLine | None:
    """ GET /logs

    Args:
        service (str | Unset):
        kind (str | Unset):
        level (str | Unset):
        tenant (str | Unset):
        call_id (str | Unset):
        q (str | Unset):
        since (str | Unset):
        until (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageLogLine
     """


    return (await asyncio_detailed(
        client=client,
service=service,
kind=kind,
level=level,
tenant=tenant,
call_id=call_id,
q=q,
since=since,
until=until,
limit=limit,
cursor=cursor,

    )).parsed
