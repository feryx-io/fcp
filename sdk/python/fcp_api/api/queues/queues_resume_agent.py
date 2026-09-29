from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.agent_response import AgentResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    name: str,
    id: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/queues/{name}/agents/{id}/resume".format(name=quote(str(name), safe=""),id=quote(str(id), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> AgentResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = AgentResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[AgentResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    name: str,
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[AgentResponse | ErrorResponse]:
    """ POST /queues/:name/agents/:id/resume — Resume a paused agent.

    Args:
        name (str):
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AgentResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        name=name,
id=id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    name: str,
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> AgentResponse | ErrorResponse | None:
    """ POST /queues/:name/agents/:id/resume — Resume a paused agent.

    Args:
        name (str):
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        AgentResponse | ErrorResponse
     """


    return sync_detailed(
        name=name,
id=id,
client=client,

    ).parsed

async def asyncio_detailed(
    name: str,
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[AgentResponse | ErrorResponse]:
    """ POST /queues/:name/agents/:id/resume — Resume a paused agent.

    Args:
        name (str):
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AgentResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        name=name,
id=id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    name: str,
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> AgentResponse | ErrorResponse | None:
    """ POST /queues/:name/agents/:id/resume — Resume a paused agent.

    Args:
        name (str):
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        AgentResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        name=name,
id=id,
client=client,

    )).parsed
