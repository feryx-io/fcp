from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.declare_location_request import DeclareLocationRequest
from ...models.declared_location_response import DeclaredLocationResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    *,
    body: DeclareLocationRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/me/location",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> DeclaredLocationResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = DeclaredLocationResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[DeclaredLocationResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: DeclareLocationRequest,

) -> Response[DeclaredLocationResponse | ErrorResponse]:
    """ `PUT /v1/me/location` — say where I am, for emergency calls. The
    address is validated like any other; it stands for the tenant's
    `declared_location_ttl_secs` (a day by default).

    Args:
        body (DeclareLocationRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DeclaredLocationResponse | ErrorResponse]
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
    body: DeclareLocationRequest,

) -> DeclaredLocationResponse | ErrorResponse | None:
    """ `PUT /v1/me/location` — say where I am, for emergency calls. The
    address is validated like any other; it stands for the tenant's
    `declared_location_ttl_secs` (a day by default).

    Args:
        body (DeclareLocationRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DeclaredLocationResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: DeclareLocationRequest,

) -> Response[DeclaredLocationResponse | ErrorResponse]:
    """ `PUT /v1/me/location` — say where I am, for emergency calls. The
    address is validated like any other; it stands for the tenant's
    `declared_location_ttl_secs` (a day by default).

    Args:
        body (DeclareLocationRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DeclaredLocationResponse | ErrorResponse]
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
    body: DeclareLocationRequest,

) -> DeclaredLocationResponse | ErrorResponse | None:
    """ `PUT /v1/me/location` — say where I am, for emergency calls. The
    address is validated like any other; it stands for the tenant's
    `declared_location_ttl_secs` (a day by default).

    Args:
        body (DeclareLocationRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DeclaredLocationResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
