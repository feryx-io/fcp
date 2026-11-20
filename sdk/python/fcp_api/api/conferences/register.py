from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.register_request import RegisterRequest
from ...models.registered_response import RegisteredResponse
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: RegisterRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/conferences/meetings/{id}/registrants".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RegisteredResponse:
    if response.status_code == 201:
        response_201 = RegisteredResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RegisteredResponse]:
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
    body: RegisterRequest,

) -> Response[ErrorResponse | RegisteredResponse]:
    """ POST /conferences/meetings/:id/registrants — register a person for a
    webinar: their registration and attendee token.

    Args:
        id (str):
        body (RegisterRequest): `POST …/registrants` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RegisteredResponse]
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
    body: RegisterRequest,

) -> ErrorResponse | RegisteredResponse | None:
    """ POST /conferences/meetings/:id/registrants — register a person for a
    webinar: their registration and attendee token.

    Args:
        id (str):
        body (RegisterRequest): `POST …/registrants` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RegisteredResponse
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
    body: RegisterRequest,

) -> Response[ErrorResponse | RegisteredResponse]:
    """ POST /conferences/meetings/:id/registrants — register a person for a
    webinar: their registration and attendee token.

    Args:
        id (str):
        body (RegisterRequest): `POST …/registrants` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RegisteredResponse]
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
    body: RegisterRequest,

) -> ErrorResponse | RegisteredResponse | None:
    """ POST /conferences/meetings/:id/registrants — register a person for a
    webinar: their registration and attendee token.

    Args:
        id (str):
        body (RegisterRequest): `POST …/registrants` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RegisteredResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
