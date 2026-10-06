from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_directory_entry import PageDirectoryEntry
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    q: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["limit"] = limit

    params["cursor"] = cursor

    params["q"] = q


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/me/directory",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageDirectoryEntry:
    if response.status_code == 200:
        response_200 = PageDirectoryEntry.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageDirectoryEntry]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    q: str | Unset = UNSET,

) -> Response[ErrorResponse | PageDirectoryEntry]:
    """ GET /me/directory — the tenant's people and the extensions that ring
    no one person, each with its presence, while the tenant lets its users
    see them (`directory.visible_to_users`; 403 otherwise).

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        q (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageDirectoryEntry]
     """


    kwargs = _get_kwargs(
        limit=limit,
cursor=cursor,
q=q,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    q: str | Unset = UNSET,

) -> ErrorResponse | PageDirectoryEntry | None:
    """ GET /me/directory — the tenant's people and the extensions that ring
    no one person, each with its presence, while the tenant lets its users
    see them (`directory.visible_to_users`; 403 otherwise).

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        q (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageDirectoryEntry
     """


    return sync_detailed(
        client=client,
limit=limit,
cursor=cursor,
q=q,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    q: str | Unset = UNSET,

) -> Response[ErrorResponse | PageDirectoryEntry]:
    """ GET /me/directory — the tenant's people and the extensions that ring
    no one person, each with its presence, while the tenant lets its users
    see them (`directory.visible_to_users`; 403 otherwise).

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        q (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageDirectoryEntry]
     """


    kwargs = _get_kwargs(
        limit=limit,
cursor=cursor,
q=q,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    q: str | Unset = UNSET,

) -> ErrorResponse | PageDirectoryEntry | None:
    """ GET /me/directory — the tenant's people and the extensions that ring
    no one person, each with its presence, while the tenant lets its users
    see them (`directory.visible_to_users`; 403 otherwise).

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        q (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageDirectoryEntry
     """


    return (await asyncio_detailed(
        client=client,
limit=limit,
cursor=cursor,
q=q,

    )).parsed
