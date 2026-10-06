from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.brand_asset_response import BrandAssetResponse
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    body: str,
    name: str | Unset = UNSET,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    params: dict[str, Any] = {}

    params["name"] = name


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/branding/assets",
        "params": params,
    }

    _kwargs["content"] = body.payload
    headers["Content-Type"] = "application/octet-stream"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> BrandAssetResponse | ErrorResponse:
    if response.status_code == 201:
        response_201 = BrandAssetResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[BrandAssetResponse | ErrorResponse]:
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
    name: str | Unset = UNSET,

) -> Response[BrandAssetResponse | ErrorResponse]:
    """ POST /api/v1/branding/assets — upload a file as the body: a PNG, JPEG,
    WebP or SVG image, or Ogg, MP3 or WAV audio, at most 5 MB. What it is
    comes from its bytes. The same file again is the same asset.

    Args:
        name (str | Unset):
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[BrandAssetResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,
name=name,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: str,
    name: str | Unset = UNSET,

) -> BrandAssetResponse | ErrorResponse | None:
    """ POST /api/v1/branding/assets — upload a file as the body: a PNG, JPEG,
    WebP or SVG image, or Ogg, MP3 or WAV audio, at most 5 MB. What it is
    comes from its bytes. The same file again is the same asset.

    Args:
        name (str | Unset):
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        BrandAssetResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,
name=name,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: str,
    name: str | Unset = UNSET,

) -> Response[BrandAssetResponse | ErrorResponse]:
    """ POST /api/v1/branding/assets — upload a file as the body: a PNG, JPEG,
    WebP or SVG image, or Ogg, MP3 or WAV audio, at most 5 MB. What it is
    comes from its bytes. The same file again is the same asset.

    Args:
        name (str | Unset):
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[BrandAssetResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,
name=name,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: str,
    name: str | Unset = UNSET,

) -> BrandAssetResponse | ErrorResponse | None:
    """ POST /api/v1/branding/assets — upload a file as the body: a PNG, JPEG,
    WebP or SVG image, or Ogg, MP3 or WAV audio, at most 5 MB. What it is
    comes from its bytes. The same file again is the same asset.

    Args:
        name (str | Unset):
        body (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        BrandAssetResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,
name=name,

    )).parsed
