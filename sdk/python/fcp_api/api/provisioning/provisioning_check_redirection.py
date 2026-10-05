from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.redirection_check_response import RedirectionCheckResponse
from typing import cast



def _get_kwargs(
    service: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/provisioning/redirection/{service}/check".format(service=quote(str(service), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RedirectionCheckResponse:
    if response.status_code == 200:
        response_200 = RedirectionCheckResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RedirectionCheckResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RedirectionCheckResponse]:
    """ POST /api/v1/provisioning/redirection/:service/check — ask the service
    whether the account's credentials work.

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionCheckResponse]
     """


    kwargs = _get_kwargs(
        service=service,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RedirectionCheckResponse | None:
    """ POST /api/v1/provisioning/redirection/:service/check — ask the service
    whether the account's credentials work.

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionCheckResponse
     """


    return sync_detailed(
        service=service,
client=client,

    ).parsed

async def asyncio_detailed(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | RedirectionCheckResponse]:
    """ POST /api/v1/provisioning/redirection/:service/check — ask the service
    whether the account's credentials work.

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionCheckResponse]
     """


    kwargs = _get_kwargs(
        service=service,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    service: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | RedirectionCheckResponse | None:
    """ POST /api/v1/provisioning/redirection/:service/check — ask the service
    whether the account's credentials work.

    Args:
        service (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionCheckResponse
     """


    return (await asyncio_detailed(
        service=service,
client=client,

    )).parsed
