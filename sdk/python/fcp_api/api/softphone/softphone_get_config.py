from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.softphone_config import SoftphoneConfig
from typing import cast



def _get_kwargs(
    
) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/me/softphone",
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse | SoftphoneConfig:
    if response.status_code == 200:
        response_200 = SoftphoneConfig.from_dict(response.json())



        return response_200

    if response.status_code == 304:
        response_304 = cast(Any, None)
        return response_304

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[Any | ErrorResponse | SoftphoneConfig]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse | SoftphoneConfig]:
    """ GET /api/v1/me/softphone — the app's SIP account and settings, with an
    ETag (`If-None-Match` answers 304).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse | SoftphoneConfig]
     """


    kwargs = _get_kwargs(
        
    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | SoftphoneConfig | None:
    """ GET /api/v1/me/softphone — the app's SIP account and settings, with an
    ETag (`If-None-Match` answers 304).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse | SoftphoneConfig
     """


    return sync_detailed(
        client=client,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse | SoftphoneConfig]:
    """ GET /api/v1/me/softphone — the app's SIP account and settings, with an
    ETag (`If-None-Match` answers 304).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse | SoftphoneConfig]
     """


    kwargs = _get_kwargs(
        
    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | SoftphoneConfig | None:
    """ GET /api/v1/me/softphone — the app's SIP account and settings, with an
    ETag (`If-None-Match` answers 304).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse | SoftphoneConfig
     """


    return (await asyncio_detailed(
        client=client,

    )).parsed
