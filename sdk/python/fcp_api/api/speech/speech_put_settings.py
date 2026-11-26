from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.speech_settings_request import SpeechSettingsRequest
from ...models.speech_settings_response import SpeechSettingsResponse
from typing import cast



def _get_kwargs(
    *,
    body: SpeechSettingsRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/speech/settings",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | SpeechSettingsResponse:
    if response.status_code == 200:
        response_200 = SpeechSettingsResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | SpeechSettingsResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: SpeechSettingsRequest,

) -> Response[ErrorResponse | SpeechSettingsResponse]:
    """ PUT /speech/settings

    Args:
        body (SpeechSettingsRequest): The settings to keep; null or absent is the platform's
            default.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SpeechSettingsResponse]
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
    body: SpeechSettingsRequest,

) -> ErrorResponse | SpeechSettingsResponse | None:
    """ PUT /speech/settings

    Args:
        body (SpeechSettingsRequest): The settings to keep; null or absent is the platform's
            default.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SpeechSettingsResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: SpeechSettingsRequest,

) -> Response[ErrorResponse | SpeechSettingsResponse]:
    """ PUT /speech/settings

    Args:
        body (SpeechSettingsRequest): The settings to keep; null or absent is the platform's
            default.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SpeechSettingsResponse]
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
    body: SpeechSettingsRequest,

) -> ErrorResponse | SpeechSettingsResponse | None:
    """ PUT /speech/settings

    Args:
        body (SpeechSettingsRequest): The settings to keep; null or absent is the platform's
            default.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SpeechSettingsResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
