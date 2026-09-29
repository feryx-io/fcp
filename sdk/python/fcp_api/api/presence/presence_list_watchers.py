from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_watcher_response import PageWatcherResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    aor: str,
    *,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/presence/{aor}/watchers".format(aor=quote(str(aor), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageWatcherResponse:
    if response.status_code == 200:
        response_200 = PageWatcherResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageWatcherResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageWatcherResponse]:
    """ GET /presence/:aor/watchers — List SIP watchers for an AOR.

    Args:
        aor (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageWatcherResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,
limit=limit,
cursor=cursor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageWatcherResponse | None:
    """ GET /presence/:aor/watchers — List SIP watchers for an AOR.

    Args:
        aor (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageWatcherResponse
     """


    return sync_detailed(
        aor=aor,
client=client,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageWatcherResponse]:
    """ GET /presence/:aor/watchers — List SIP watchers for an AOR.

    Args:
        aor (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageWatcherResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,
limit=limit,
cursor=cursor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageWatcherResponse | None:
    """ GET /presence/:aor/watchers — List SIP watchers for an AOR.

    Args:
        aor (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageWatcherResponse
     """


    return (await asyncio_detailed(
        aor=aor,
client=client,
limit=limit,
cursor=cursor,

    )).parsed
