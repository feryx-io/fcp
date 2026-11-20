from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.registrant_response import RegistrantResponse
from typing import cast



def _get_kwargs(
    id: str,
    rid: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "delete",
        "url": "/api/v1/conferences/meetings/{id}/registrants/{rid}".format(id=quote(str(id), safe=""),rid=quote(str(rid), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RegistrantResponse:
    if response.status_code == 200:
        response_200 = RegistrantResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RegistrantResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RegistrantResponse]:
    """ DELETE /conferences/meetings/:id/registrants/:rid — revoke a
    registration: its token is refused from now on. Someone already in
    the room stays until a host removes them.

    Args:
        id (str):
        rid (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RegistrantResponse]
     """


    kwargs = _get_kwargs(
        id=id,
rid=rid,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RegistrantResponse | None:
    """ DELETE /conferences/meetings/:id/registrants/:rid — revoke a
    registration: its token is refused from now on. Someone already in
    the room stays until a host removes them.

    Args:
        id (str):
        rid (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RegistrantResponse
     """


    return sync_detailed(
        id=id,
rid=rid,
client=client,

    ).parsed

async def asyncio_detailed(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RegistrantResponse]:
    """ DELETE /conferences/meetings/:id/registrants/:rid — revoke a
    registration: its token is refused from now on. Someone already in
    the room stays until a host removes them.

    Args:
        id (str):
        rid (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RegistrantResponse]
     """


    kwargs = _get_kwargs(
        id=id,
rid=rid,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RegistrantResponse | None:
    """ DELETE /conferences/meetings/:id/registrants/:rid — revoke a
    registration: its token is refused from now on. Someone already in
    the room stays until a host removes them.

    Args:
        id (str):
        rid (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RegistrantResponse
     """


    return (await asyncio_detailed(
        id=id,
rid=rid,
client=client,

    )).parsed
