from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.platform_speech_cap_request import PlatformSpeechCapRequest
from typing import cast



def _get_kwargs(
    provider: str,
    *,
    body: PlatformSpeechCapRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/system/speech/caps/{provider}".format(provider=quote(str(provider), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
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
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    body: PlatformSpeechCapRequest,

) -> Response[Any | ErrorResponse]:
    """ PUT /system/speech/caps/{provider} — a platform provider's default cap.

    Args:
        provider (str):
        body (PlatformSpeechCapRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        provider=provider,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    body: PlatformSpeechCapRequest,

) -> Any | ErrorResponse | None:
    """ PUT /system/speech/caps/{provider} — a platform provider's default cap.

    Args:
        provider (str):
        body (PlatformSpeechCapRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        provider=provider,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    body: PlatformSpeechCapRequest,

) -> Response[Any | ErrorResponse]:
    """ PUT /system/speech/caps/{provider} — a platform provider's default cap.

    Args:
        provider (str):
        body (PlatformSpeechCapRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        provider=provider,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    body: PlatformSpeechCapRequest,

) -> Any | ErrorResponse | None:
    """ PUT /system/speech/caps/{provider} — a platform provider's default cap.

    Args:
        provider (str):
        body (PlatformSpeechCapRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        provider=provider,
client=client,
body=body,

    )).parsed
