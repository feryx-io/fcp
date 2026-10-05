from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.firmware_image_response import FirmwareImageResponse
from typing import cast



def _get_kwargs(
    *,
    body: str,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/firmware",
    }

    
    headers["Content-Type"] = "multipart/form-data; boundary=+++"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | FirmwareImageResponse:
    if response.status_code == 201:
        response_201 = FirmwareImageResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | FirmwareImageResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: str,

) -> Response[ErrorResponse | FirmwareImageResponse]:
    """ POST /api/v1/firmware — upload an image (multipart: `vendor`,
    `version`, `models` (comma-separated patterns), `notes`, and `file`,
    whose name is the one it is served under). The platform
    administrator's.

    Args:
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | FirmwareImageResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: str,

) -> ErrorResponse | FirmwareImageResponse | None:
    """ POST /api/v1/firmware — upload an image (multipart: `vendor`,
    `version`, `models` (comma-separated patterns), `notes`, and `file`,
    whose name is the one it is served under). The platform
    administrator's.

    Args:
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | FirmwareImageResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: str,

) -> Response[ErrorResponse | FirmwareImageResponse]:
    """ POST /api/v1/firmware — upload an image (multipart: `vendor`,
    `version`, `models` (comma-separated patterns), `notes`, and `file`,
    whose name is the one it is served under). The platform
    administrator's.

    Args:
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | FirmwareImageResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: str,

) -> ErrorResponse | FirmwareImageResponse | None:
    """ POST /api/v1/firmware — upload an image (multipart: `vendor`,
    `version`, `models` (comma-separated patterns), `notes`, and `file`,
    whose name is the one it is served under). The platform
    administrator's.

    Args:
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | FirmwareImageResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
