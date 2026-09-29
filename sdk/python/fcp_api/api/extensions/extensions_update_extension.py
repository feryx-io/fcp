from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.extension_response import ExtensionResponse
from ...models.update_extension_request import UpdateExtensionRequest
from typing import cast



def _get_kwargs(
    number: str,
    *,
    body: UpdateExtensionRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "patch",
        "url": "/api/v1/extensions/{number}".format(number=quote(str(number), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | ExtensionResponse:
    if response.status_code == 200:
        response_200 = ExtensionResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | ExtensionResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    number: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateExtensionRequest,

) -> Response[ErrorResponse | ExtensionResponse]:
    """ Update extension.

    Args:
        number (str):
        body (UpdateExtensionRequest): Update extension request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | ExtensionResponse]
     """


    kwargs = _get_kwargs(
        number=number,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    number: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateExtensionRequest,

) -> ErrorResponse | ExtensionResponse | None:
    """ Update extension.

    Args:
        number (str):
        body (UpdateExtensionRequest): Update extension request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | ExtensionResponse
     """


    return sync_detailed(
        number=number,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    number: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateExtensionRequest,

) -> Response[ErrorResponse | ExtensionResponse]:
    """ Update extension.

    Args:
        number (str):
        body (UpdateExtensionRequest): Update extension request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | ExtensionResponse]
     """


    kwargs = _get_kwargs(
        number=number,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    number: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateExtensionRequest,

) -> ErrorResponse | ExtensionResponse | None:
    """ Update extension.

    Args:
        number (str):
        body (UpdateExtensionRequest): Update extension request.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | ExtensionResponse
     """


    return (await asyncio_detailed(
        number=number,
client=client,
body=body,

    )).parsed
