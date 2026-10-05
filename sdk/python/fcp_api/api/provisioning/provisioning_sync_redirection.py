from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.redirection_sync_response import RedirectionSyncResponse
from typing import cast



def _get_kwargs(
    service: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/provisioning/redirection/{service}/sync".format(service=quote(str(service), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RedirectionSyncResponse:
    if response.status_code == 200:
        response_200 = RedirectionSyncResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RedirectionSyncResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RedirectionSyncResponse]:
    """ POST /api/v1/provisioning/redirection/:service/sync — register every
    phone of the service's vendor not yet registered (an account made after
    its devices).

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionSyncResponse]
     """


    kwargs = _get_kwargs(
        service=service,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RedirectionSyncResponse | None:
    """ POST /api/v1/provisioning/redirection/:service/sync — register every
    phone of the service's vendor not yet registered (an account made after
    its devices).

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionSyncResponse
     """


    return sync_detailed(
        service=service,
client=client,

    ).parsed

async def asyncio_detailed(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RedirectionSyncResponse]:
    """ POST /api/v1/provisioning/redirection/:service/sync — register every
    phone of the service's vendor not yet registered (an account made after
    its devices).

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionSyncResponse]
     """


    kwargs = _get_kwargs(
        service=service,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RedirectionSyncResponse | None:
    """ POST /api/v1/provisioning/redirection/:service/sync — register every
    phone of the service's vendor not yet registered (an account made after
    its devices).

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionSyncResponse
     """


    return (await asyncio_detailed(
        service=service,
client=client,

    )).parsed
