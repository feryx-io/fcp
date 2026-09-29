from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_agent_response import PageAgentResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    name: str,
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
        "url": "/api/v1/queues/{name}/agents".format(name=quote(str(name), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageAgentResponse:
    if response.status_code == 200:
        response_200 = PageAgentResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageAgentResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    name: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageAgentResponse]:
    """ GET /queues/:name/agents — List agents in a queue.

    Args:
        name (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageAgentResponse]
     """


    kwargs = _get_kwargs(
        name=name,
limit=limit,
cursor=cursor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    name: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageAgentResponse | None:
    """ GET /queues/:name/agents — List agents in a queue.

    Args:
        name (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageAgentResponse
     """


    return sync_detailed(
        name=name,
client=client,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    name: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageAgentResponse]:
    """ GET /queues/:name/agents — List agents in a queue.

    Args:
        name (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageAgentResponse]
     """


    kwargs = _get_kwargs(
        name=name,
limit=limit,
cursor=cursor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    name: str,
    *,
    client: AuthenticatedClient | Client,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageAgentResponse | None:
    """ GET /queues/:name/agents — List agents in a queue.

    Args:
        name (str):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageAgentResponse
     """


    return (await asyncio_detailed(
        name=name,
client=client,
limit=limit,
cursor=cursor,

    )).parsed
