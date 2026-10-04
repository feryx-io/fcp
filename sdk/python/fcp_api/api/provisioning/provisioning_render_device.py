from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.render_device_request import RenderDeviceRequest
from ...models.rendered_config_response import RenderedConfigResponse
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: RenderDeviceRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/devices/{id}/render".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RenderedConfigResponse:
    if response.status_code == 200:
        response_200 = RenderedConfigResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RenderedConfigResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RenderDeviceRequest,

) -> Response[ErrorResponse | RenderedConfigResponse]:
    """ POST /api/v1/devices/:id/render — the device's configuration as its
    phone would get it now (credentials included), from its template or
    the one given.

    Args:
        id (str):
        body (RenderDeviceRequest): A template to try, in place of the device's.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RenderedConfigResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RenderDeviceRequest,

) -> ErrorResponse | RenderedConfigResponse | None:
    """ POST /api/v1/devices/:id/render — the device's configuration as its
    phone would get it now (credentials included), from its template or
    the one given.

    Args:
        id (str):
        body (RenderDeviceRequest): A template to try, in place of the device's.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RenderedConfigResponse
     """


    return sync_detailed(
        id=id,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RenderDeviceRequest,

) -> Response[ErrorResponse | RenderedConfigResponse]:
    """ POST /api/v1/devices/:id/render — the device's configuration as its
    phone would get it now (credentials included), from its template or
    the one given.

    Args:
        id (str):
        body (RenderDeviceRequest): A template to try, in place of the device's.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RenderedConfigResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: RenderDeviceRequest,

) -> ErrorResponse | RenderedConfigResponse | None:
    """ POST /api/v1/devices/:id/render — the device's configuration as its
    phone would get it now (credentials included), from its template or
    the one given.

    Args:
        id (str):
        body (RenderDeviceRequest): A template to try, in place of the device's.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RenderedConfigResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
