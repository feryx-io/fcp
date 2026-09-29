from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.cluster_nodes_response import ClusterNodesResponse
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    region: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["region"] = region


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/cluster/nodes",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ClusterNodesResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ClusterNodesResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ClusterNodesResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    region: str | Unset = UNSET,

) -> Response[ClusterNodesResponse | ErrorResponse]:
    """ 
    Args:
        region (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClusterNodesResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        region=region,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    region: str | Unset = UNSET,

) -> ClusterNodesResponse | ErrorResponse | None:
    """ 
    Args:
        region (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClusterNodesResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
region=region,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    region: str | Unset = UNSET,

) -> Response[ClusterNodesResponse | ErrorResponse]:
    """ 
    Args:
        region (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClusterNodesResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        region=region,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    region: str | Unset = UNSET,

) -> ClusterNodesResponse | ErrorResponse | None:
    """ 
    Args:
        region (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClusterNodesResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
region=region,

    )).parsed
