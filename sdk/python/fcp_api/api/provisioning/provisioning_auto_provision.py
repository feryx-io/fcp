from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    mac_address: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/provisioning/{mac_address}".format(mac_address=quote(str(mac_address), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | str:
    if response.status_code == 200:
        response_200 = response.text
        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | str]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    mac_address: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | str]:
    """ GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.

     Phones must present both:
    - `Authorization: Bearer <token>`
    - or `X-FCP-Provisioning-Token: <token>`
    - and `X-FCP-Device-Provisioning-Token: <per-device-token>`

    Unknown or unauthorized devices all return the same 404 to avoid turning
    this endpoint into an enumeration surface.

    Args:
        mac_address (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | str]
     """


    kwargs = _get_kwargs(
        mac_address=mac_address,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    mac_address: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | str | None:
    """ GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.

     Phones must present both:
    - `Authorization: Bearer <token>`
    - or `X-FCP-Provisioning-Token: <token>`
    - and `X-FCP-Device-Provisioning-Token: <per-device-token>`

    Unknown or unauthorized devices all return the same 404 to avoid turning
    this endpoint into an enumeration surface.

    Args:
        mac_address (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | str
     """


    return sync_detailed(
        mac_address=mac_address,
client=client,

    ).parsed

async def asyncio_detailed(
    mac_address: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | str]:
    """ GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.

     Phones must present both:
    - `Authorization: Bearer <token>`
    - or `X-FCP-Provisioning-Token: <token>`
    - and `X-FCP-Device-Provisioning-Token: <per-device-token>`

    Unknown or unauthorized devices all return the same 404 to avoid turning
    this endpoint into an enumeration surface.

    Args:
        mac_address (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | str]
     """


    kwargs = _get_kwargs(
        mac_address=mac_address,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    mac_address: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | str | None:
    """ GET /provisioning/:mac_address -- Auto-provision a phone by MAC address.

     Phones must present both:
    - `Authorization: Bearer <token>`
    - or `X-FCP-Provisioning-Token: <token>`
    - and `X-FCP-Device-Provisioning-Token: <per-device-token>`

    Unknown or unauthorized devices all return the same 404 to avoid turning
    this endpoint into an enumeration surface.

    Args:
        mac_address (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | str
     """


    return (await asyncio_detailed(
        mac_address=mac_address,
client=client,

    )).parsed
