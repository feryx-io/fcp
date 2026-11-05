from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_deletion_response import PageDeletionResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    class_: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    request_id: str | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["class"] = class_

    params["since"] = since

    params["until"] = until

    params["request_id"] = request_id

    params["api_request_id"] = api_request_id

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/compliance/deletions",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageDeletionResponse:
    if response.status_code == 200:
        response_200 = PageDeletionResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageDeletionResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    class_: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    request_id: str | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageDeletionResponse]:
    """ `GET /compliance/deletions`: what was deleted, when, why and by whom
    (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
    platform token sees every tenant, platform upkeep included, unless it
    selected one.

    Args:
        class_ (str | Unset):
        since (int | Unset):
        until (int | Unset):
        request_id (str | Unset):
        api_request_id (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageDeletionResponse]
     """


    kwargs = _get_kwargs(
        class_=class_,
since=since,
until=until,
request_id=request_id,
api_request_id=api_request_id,
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
    class_: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    request_id: str | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageDeletionResponse | None:
    """ `GET /compliance/deletions`: what was deleted, when, why and by whom
    (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
    platform token sees every tenant, platform upkeep included, unless it
    selected one.

    Args:
        class_ (str | Unset):
        since (int | Unset):
        until (int | Unset):
        request_id (str | Unset):
        api_request_id (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageDeletionResponse
     """


    return sync_detailed(
        client=client,
class_=class_,
since=since,
until=until,
request_id=request_id,
api_request_id=api_request_id,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    class_: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    request_id: str | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageDeletionResponse]:
    """ `GET /compliance/deletions`: what was deleted, when, why and by whom
    (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
    platform token sees every tenant, platform upkeep included, unless it
    selected one.

    Args:
        class_ (str | Unset):
        since (int | Unset):
        until (int | Unset):
        request_id (str | Unset):
        api_request_id (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageDeletionResponse]
     """


    kwargs = _get_kwargs(
        class_=class_,
since=since,
until=until,
request_id=request_id,
api_request_id=api_request_id,
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
    class_: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    request_id: str | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageDeletionResponse | None:
    """ `GET /compliance/deletions`: what was deleted, when, why and by whom
    (`docs/COMPLIANCE_FOLLOWUPS.md` §7). Tenant admins see their tenant; the
    platform token sees every tenant, platform upkeep included, unless it
    selected one.

    Args:
        class_ (str | Unset):
        since (int | Unset):
        until (int | Unset):
        request_id (str | Unset):
        api_request_id (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageDeletionResponse
     """


    return (await asyncio_detailed(
        client=client,
class_=class_,
since=since,
until=until,
request_id=request_id,
api_request_id=api_request_id,
limit=limit,
cursor=cursor,

    )).parsed
