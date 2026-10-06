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
    sha256: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "delete",
        "url": "/api/v1/branding/assets/{sha256}".format(sha256=quote(str(sha256), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 204:
        response_204 = cast(Any, None)
        return response_204

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[Any | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    sha256: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse]:
    """ DELETE /api/v1/branding/assets/{sha256} — one the brand names is
    refused (409).

    Args:
        sha256 (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        sha256=sha256,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    sha256: str,
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | None:
    """ DELETE /api/v1/branding/assets/{sha256} — one the brand names is
    refused (409).

    Args:
        sha256 (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        sha256=sha256,
client=client,

    ).parsed

async def asyncio_detailed(
    sha256: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse]:
    """ DELETE /api/v1/branding/assets/{sha256} — one the brand names is
    refused (409).

    Args:
        sha256 (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        sha256=sha256,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    sha256: str,
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | None:
    """ DELETE /api/v1/branding/assets/{sha256} — one the brand names is
    refused (409).

    Args:
        sha256 (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        sha256=sha256,
client=client,

    )).parsed
