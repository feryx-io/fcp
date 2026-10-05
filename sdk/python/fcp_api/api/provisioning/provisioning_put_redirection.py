from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.put_redirection_account_request import PutRedirectionAccountRequest
from ...models.redirection_account_response import RedirectionAccountResponse
from typing import cast



def _get_kwargs(
    service: str,
    *,
    body: PutRedirectionAccountRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/provisioning/redirection/{service}".format(service=quote(str(service), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | RedirectionAccountResponse:
    if response.status_code == 200:
        response_200 = RedirectionAccountResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | RedirectionAccountResponse]:
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
    body: PutRedirectionAccountRequest,

) -> Response[ErrorResponse | RedirectionAccountResponse]:
    """ PUT /api/v1/provisioning/redirection/:service — the tenant's account.

    Args:
        service (str):
        body (PutRedirectionAccountRequest): An account, made or changed.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionAccountResponse]
     """


    kwargs = _get_kwargs(
        service=service,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    service: str,
    *,
    client: AuthenticatedClient | Client,
    body: PutRedirectionAccountRequest,

) -> ErrorResponse | RedirectionAccountResponse | None:
    """ PUT /api/v1/provisioning/redirection/:service — the tenant's account.

    Args:
        service (str):
        body (PutRedirectionAccountRequest): An account, made or changed.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionAccountResponse
     """


    return sync_detailed(
        service=service,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    service: str,
    *,
    client: AuthenticatedClient | Client,
    body: PutRedirectionAccountRequest,

) -> Response[ErrorResponse | RedirectionAccountResponse]:
    """ PUT /api/v1/provisioning/redirection/:service — the tenant's account.

    Args:
        service (str):
        body (PutRedirectionAccountRequest): An account, made or changed.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | RedirectionAccountResponse]
     """


    kwargs = _get_kwargs(
        service=service,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    service: str,
    *,
    client: AuthenticatedClient | Client,
    body: PutRedirectionAccountRequest,

) -> ErrorResponse | RedirectionAccountResponse | None:
    """ PUT /api/v1/provisioning/redirection/:service — the tenant's account.

    Args:
        service (str):
        body (PutRedirectionAccountRequest): An account, made or changed.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | RedirectionAccountResponse
     """


    return (await asyncio_detailed(
        service=service,
client=client,
body=body,

    )).parsed
