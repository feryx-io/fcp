from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.location_validation_response import LocationValidationResponse
from typing import cast



def _get_kwargs(
    id: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/locations/{id}/e911/validate".format(id=quote(str(id), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | LocationValidationResponse:
    if response.status_code == 200:
        response_200 = LocationValidationResponse.from_dict(response.json())



        return response_200

    if response.status_code == 422:
        response_422 = LocationValidationResponse.from_dict(response.json())



        return response_422

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | LocationValidationResponse]:
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

) -> Response[ErrorResponse | LocationValidationResponse]:
    """ Validate a site's address and mark the site validated on success. The
    normalised street, city, state and civic elements are written back, so
    the emergency calls placed from this site carry them
    (`docs/E911_BLOCK_2.md` §7).

    Args:
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | LocationValidationResponse]
     """


    kwargs = _get_kwargs(
        id=id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | LocationValidationResponse | None:
    """ Validate a site's address and mark the site validated on success. The
    normalised street, city, state and civic elements are written back, so
    the emergency calls placed from this site carry them
    (`docs/E911_BLOCK_2.md` §7).

    Args:
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | LocationValidationResponse
     """


    return sync_detailed(
        id=id,
client=client,

    ).parsed

async def asyncio_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | LocationValidationResponse]:
    """ Validate a site's address and mark the site validated on success. The
    normalised street, city, state and civic elements are written back, so
    the emergency calls placed from this site carry them
    (`docs/E911_BLOCK_2.md` §7).

    Args:
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | LocationValidationResponse]
     """


    kwargs = _get_kwargs(
        id=id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | LocationValidationResponse | None:
    """ Validate a site's address and mark the site validated on success. The
    normalised street, city, state and civic elements are written back, so
    the emergency calls placed from this site carry them
    (`docs/E911_BLOCK_2.md` §7).

    Args:
        id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | LocationValidationResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,

    )).parsed
