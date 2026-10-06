from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_my_call_response import PageMyCallResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    missed: bool | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["limit"] = limit

    params["cursor"] = cursor

    params["missed"] = missed


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/me/calls",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageMyCallResponse:
    if response.status_code == 200:
        response_200 = PageMyCallResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageMyCallResponse]:
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
    missed: bool | Unset = UNSET,

) -> Response[ErrorResponse | PageMyCallResponse]:
    """ GET /me/calls — the user's calls, newest first: the records whose
    caller or callee is one of their names, extensions, numbers or
    addresses, as a subject export finds them.

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        missed (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageMyCallResponse]
     """


    kwargs = _get_kwargs(
        limit=limit,
cursor=cursor,
missed=missed,

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
    missed: bool | Unset = UNSET,

) -> ErrorResponse | PageMyCallResponse | None:
    """ GET /me/calls — the user's calls, newest first: the records whose
    caller or callee is one of their names, extensions, numbers or
    addresses, as a subject export finds them.

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        missed (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageMyCallResponse
     """


    return sync_detailed(
        client=client,
limit=limit,
cursor=cursor,
missed=missed,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,
    missed: bool | Unset = UNSET,

) -> Response[ErrorResponse | PageMyCallResponse]:
    """ GET /me/calls — the user's calls, newest first: the records whose
    caller or callee is one of their names, extensions, numbers or
    addresses, as a subject export finds them.

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        missed (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageMyCallResponse]
     """


    kwargs = _get_kwargs(
        limit=limit,
cursor=cursor,
missed=missed,

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
    missed: bool | Unset = UNSET,

) -> ErrorResponse | PageMyCallResponse | None:
    """ GET /me/calls — the user's calls, newest first: the records whose
    caller or callee is one of their names, extensions, numbers or
    addresses, as a subject export finds them.

    Args:
        limit (int | Unset):
        cursor (str | Unset):
        missed (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageMyCallResponse
     """


    return (await asyncio_detailed(
        client=client,
limit=limit,
cursor=cursor,
missed=missed,

    )).parsed
