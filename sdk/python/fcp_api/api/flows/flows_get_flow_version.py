from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.flow_response import FlowResponse
from typing import cast



def _get_kwargs(
    id: str,
    version: int,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/flows/{id}/versions/{version}".format(id=quote(str(id), safe=""),version=quote(str(version), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | FlowResponse:
    if response.status_code == 200:
        response_200 = FlowResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | FlowResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    version: int,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | FlowResponse]:
    """ One version of a flow: what a call that ran it ran (a flow trace names
    the version).

     GET /api/v1/flows/:id/versions/:version

    Args:
        id (str):
        version (int):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | FlowResponse]
     """


    kwargs = _get_kwargs(
        id=id,
version=version,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    version: int,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | FlowResponse | None:
    """ One version of a flow: what a call that ran it ran (a flow trace names
    the version).

     GET /api/v1/flows/:id/versions/:version

    Args:
        id (str):
        version (int):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | FlowResponse
     """


    return sync_detailed(
        id=id,
version=version,
client=client,

    ).parsed

async def asyncio_detailed(
    id: str,
    version: int,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | FlowResponse]:
    """ One version of a flow: what a call that ran it ran (a flow trace names
    the version).

     GET /api/v1/flows/:id/versions/:version

    Args:
        id (str):
        version (int):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | FlowResponse]
     """


    kwargs = _get_kwargs(
        id=id,
version=version,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    version: int,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | FlowResponse | None:
    """ One version of a flow: what a call that ran it ran (a flow trace names
    the version).

     GET /api/v1/flows/:id/versions/:version

    Args:
        id (str):
        version (int):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | FlowResponse
     """


    return (await asyncio_detailed(
        id=id,
version=version,
client=client,

    )).parsed
