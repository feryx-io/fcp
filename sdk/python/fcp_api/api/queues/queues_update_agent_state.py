from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.agent_response import AgentResponse
from ...models.error_response import ErrorResponse
from ...models.update_agent_state_request import UpdateAgentStateRequest
from typing import cast



def _get_kwargs(
    name: str,
    id: str,
    *,
    body: UpdateAgentStateRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "patch",
        "url": "/api/v1/queues/{name}/agents/{id}".format(name=quote(str(name), safe=""),id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
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
    body: UpdateAgentStateRequest,

) -> Response[AgentResponse | ErrorResponse]:
    """ PATCH /queues/:name/agents/:id — Update agent state.

    Args:
        name (str):
        id (str):
        body (UpdateAgentStateRequest): Update agent state request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AgentResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        name=name,
id=id,
body=body,

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
    body: UpdateAgentStateRequest,

) -> AgentResponse | ErrorResponse | None:
    """ PATCH /queues/:name/agents/:id — Update agent state.

    Args:
        name (str):
        id (str):
        body (UpdateAgentStateRequest): Update agent state request body.

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
body=body,

    ).parsed

async def asyncio_detailed(
    name: str,
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateAgentStateRequest,

) -> Response[AgentResponse | ErrorResponse]:
    """ PATCH /queues/:name/agents/:id — Update agent state.

    Args:
        name (str):
        id (str):
        body (UpdateAgentStateRequest): Update agent state request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AgentResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        name=name,
id=id,
body=body,

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
    body: UpdateAgentStateRequest,

) -> AgentResponse | ErrorResponse | None:
    """ PATCH /queues/:name/agents/:id — Update agent state.

    Args:
        name (str):
        id (str):
        body (UpdateAgentStateRequest): Update agent state request body.

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
body=body,

    )).parsed
