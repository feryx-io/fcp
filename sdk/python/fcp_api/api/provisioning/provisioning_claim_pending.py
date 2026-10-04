from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.claim_device_request import ClaimDeviceRequest
from ...models.device_response import DeviceResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    mac: str,
    *,
    body: ClaimDeviceRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/provisioning/pending/{mac}/claim".format(mac=quote(str(mac), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> DeviceResponse | ErrorResponse:
    if response.status_code == 201:
        response_201 = DeviceResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[DeviceResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    mac: str,
    *,
    client: AuthenticatedClient | Client,
    body: ClaimDeviceRequest,

) -> Response[DeviceResponse | ErrorResponse]:
    """ POST /api/v1/provisioning/pending/:mac/claim — make the pending phone a
    device in one step: its vendor and model as it gave them, an owner, a
    template if one is named.

    Args:
        mac (str):
        body (ClaimDeviceRequest): What a claimed or imported phone becomes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DeviceResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        mac=mac,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    mac: str,
    *,
    client: AuthenticatedClient | Client,
    body: ClaimDeviceRequest,

) -> DeviceResponse | ErrorResponse | None:
    """ POST /api/v1/provisioning/pending/:mac/claim — make the pending phone a
    device in one step: its vendor and model as it gave them, an owner, a
    template if one is named.

    Args:
        mac (str):
        body (ClaimDeviceRequest): What a claimed or imported phone becomes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DeviceResponse | ErrorResponse
     """


    return sync_detailed(
        mac=mac,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    mac: str,
    *,
    client: AuthenticatedClient | Client,
    body: ClaimDeviceRequest,

) -> Response[DeviceResponse | ErrorResponse]:
    """ POST /api/v1/provisioning/pending/:mac/claim — make the pending phone a
    device in one step: its vendor and model as it gave them, an owner, a
    template if one is named.

    Args:
        mac (str):
        body (ClaimDeviceRequest): What a claimed or imported phone becomes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DeviceResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        mac=mac,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    mac: str,
    *,
    client: AuthenticatedClient | Client,
    body: ClaimDeviceRequest,

) -> DeviceResponse | ErrorResponse | None:
    """ POST /api/v1/provisioning/pending/:mac/claim — make the pending phone a
    device in one step: its vendor and model as it gave them, an owner, a
    template if one is named.

    Args:
        mac (str):
        body (ClaimDeviceRequest): What a claimed or imported phone becomes.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DeviceResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        mac=mac,
client=client,
body=body,

    )).parsed
